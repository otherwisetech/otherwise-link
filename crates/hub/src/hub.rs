//! The server: configuration, child management, routing.
use crate::child::{CallError, ChildServer, ChildSpec};
use crate::{rpc, tools};
use otherwise_link::Root;
use otherwise_link::apps::{self, AppSpec, Capability, Installed, Locations};
use otherwise_link::link::Link;
use otherwise_link::open::Opened;
use otherwise_link::settings::HubSettings;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

pub type Opener = Arc<dyn Fn(&Link) -> std::io::Result<Opened> + Send + Sync>;

pub struct Config {
    pub root: Root,
    pub children: Vec<ChildSpec>,
    /// None: follow `hub.json` (read at start and again on every write call). Some: fixed.
    pub writes: Option<bool>,
    pub start_timeout: Duration,
    pub call_timeout: Duration,
    /// How `open_link` opens links (tests swap it).
    pub opener: Opener,
}

impl Config {
    /// The shared folder, the installed apps that have an MCP server, `hub.json`.
    pub fn system() -> Config {
        let root = Root::system();
        let settings = HubSettings::load(&root);
        let found = apps::discover(&Locations::system());
        Config::new(root.clone(), children_for(&settings, &found))
    }

    pub fn new(root: Root, children: Vec<ChildSpec>) -> Config {
        Config { root, children, writes: None, start_timeout: Duration::from_secs(15), call_timeout: Duration::from_secs(120), opener: Arc::new(|l: &Link| otherwise_link::open::open_link(l)) }
    }
}

/// Which installed apps the hub starts, and how: `<cli> mcp` (+ the write flag when writes are on).
pub fn children_for(settings: &HubSettings, found: &[(AppSpec, Installed)]) -> Vec<ChildSpec> {
    found
        .iter()
        .filter(|(s, _)| s.has(Capability::Mcp) && s.id != apps::HUB_ID && settings.mcp_enabled(&s.id))
        .filter_map(|(s, i)| {
            let program = i.cli.clone()?;
            let mut args = s.mcp_args.clone();
            let mut write_arg = None;
            if settings.mcp_writes
                && let Some(w) = &s.mcp_write_arg
            {
                args.push(w.clone());
                write_arg = Some(w.clone());
            }
            Some(ChildSpec { app: s.id.clone(), name: s.name.clone(), program, args, write_arg })
        })
        .collect()
}

struct Slot {
    spec: ChildSpec,
    server: Option<ChildServer>,
    /// The tools it listed when it last started (kept while it is down).
    tools: Vec<Value>,
    error: Option<String>,
    /// It refused its write flag and runs read-only (why), decided once per hub session: it is
    /// restarted read-only and not asked again until the hub starts again.
    read_only: Option<String>,
}

/// Starts a child; if it fails in write mode (exits, closes, or refuses `initialize`), starts it
/// again without the write flag. Never the other way round. Returns the spec that ran.
fn start_child(spec: ChildSpec, timeout: Duration) -> (Result<ChildServer, CallError>, ChildSpec, Option<String>) {
    let first = ChildServer::start(&spec, timeout);
    let refused = match &first {
        Err(e @ (CallError::Exited { .. } | CallError::Gone | CallError::Rpc { .. })) => e.to_string(),
        _ => return (first, spec, None),
    };
    let Some(ro) = spec.without_writes() else { return (first, spec, None) };
    match ChildServer::start(&ro, timeout) {
        Ok(s) => {
            eprintln!("otherwise-hub: {} refused write mode, running it read-only ({refused})", spec.name);
            (Ok(s), ro, Some(refused))
        }
        Err(e) => {
            eprintln!("otherwise-hub: {} failed to start with writes ({refused}) and read-only ({e})", spec.name);
            (Err(CallError::Io(format!("with writes: {refused}; read-only: {e}"))), spec, None)
        }
    }
}

pub struct Hub {
    pub cfg: Config,
    slots: Vec<Slot>,
    /// Writes were on when the children started (so they got their write flag).
    writes_at_start: bool,
}

/// What `otherwise-hub list` prints per app.
#[derive(Debug, Clone, PartialEq)]
pub struct ChildStatus {
    pub app: String,
    pub name: String,
    pub program: String,
    pub tools: Result<usize, String>,
    /// Running read-only because the app refused write mode (its message).
    pub read_only: Option<String>,
}

fn read_only(tool: &Value) -> bool {
    tool.pointer("/annotations/readOnlyHint").and_then(Value::as_bool) == Some(true)
}

impl Slot {
    /// The tools offered from this app: reads, plus writes when the hub offers writes and the
    /// app runs in write mode.
    fn offered(&self, writes: bool) -> impl Iterator<Item = &Value> {
        let writes = writes && self.read_only.is_none();
        self.tools.iter().filter(move |t| writes || read_only(t))
    }
}

/// `vigia.due` / `vigia__due` → ("vigia", "due").
pub fn split_name(name: &str) -> Option<(&str, &str)> {
    let (a, t) = name.split_once('.').or_else(|| name.split_once("__"))?;
    (!a.is_empty() && !t.is_empty()).then_some((a, t))
}

impl Hub {
    /// Starts every child in parallel; the ones that fail are kept with their error.
    pub fn start(cfg: Config) -> Hub {
        let writes_at_start = cfg.writes.unwrap_or_else(|| HubSettings::load(&cfg.root).mcp_writes);
        let timeout = cfg.start_timeout;
        let handles: Vec<_> = cfg
            .children
            .iter()
            .cloned()
            // writes off for this session: no child gets its write flag, whatever the spec says
            .map(|spec| if writes_at_start { spec } else { spec.without_writes().unwrap_or(spec) })
            .map(|spec| std::thread::spawn(move || start_child(spec, timeout)))
            .collect();
        let slots = handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .map(|(res, spec, read_only)| match res {
                Ok(s) => Slot { spec, tools: s.tools.clone(), server: Some(s), error: None, read_only },
                Err(e) => Slot { spec, server: None, tools: vec![], error: Some(e.to_string()), read_only },
            })
            .collect();
        Hub { cfg, slots, writes_at_start }
    }

    /// Writes allowed right now.
    pub fn writes(&self) -> bool {
        self.cfg.writes.unwrap_or_else(|| HubSettings::load(&self.cfg.root).mcp_writes)
    }

    fn offered_writes(&self) -> bool {
        self.writes_at_start && self.writes()
    }

    pub fn status(&self) -> Vec<ChildStatus> {
        self.slots
            .iter()
            .map(|s| ChildStatus {
                app: s.spec.app.clone(),
                name: s.spec.name.clone(),
                program: s.spec.program.display().to_string(),
                tools: if s.tools.is_empty() && s.server.is_none() { Err(s.error.clone().unwrap_or_default()) } else { Ok(s.tools.len()) },
                read_only: s.read_only.clone(),
            })
            .collect()
    }

    pub fn tool_list(&self) -> Vec<Value> {
        let writes = self.offered_writes();
        let mut out = tools::own(writes);
        for s in &self.slots {
            for t in s.offered(writes) {
                let Some(name) = t.get("name").and_then(Value::as_str) else { continue };
                let mut t = t.clone();
                t["name"] = json!(format!("{}.{name}", s.spec.app));
                let desc = t.get("description").and_then(Value::as_str).unwrap_or("").to_string();
                t["description"] = json!(format!("[{}] {desc}", s.spec.name));
                out.push(t);
            }
        }
        out
    }

    fn instructions(&self) -> String {
        let apps: Vec<String> = self.slots.iter().filter(|s| !s.tools.is_empty()).map(|s| format!("{} ({}.*{})", s.spec.name, s.spec.app, if s.read_only.is_some() && self.offered_writes() { ", read-only: its own settings don't allow agent changes" } else { "" })).collect();
        let mode = if self.offered_writes() { "Write tools are enabled; every write is recorded by the app that makes it." } else { "Read-only: write tools are off (the user can enable them in Otherwise Apps → Hub)." };
        let apps = if apps.is_empty() { "No app MCP servers were found.".to_string() } else { format!("App tools: {}.", apps.join(", ")) };
        format!("Otherwise Technologies' desktop apps in one place. {apps} Own tools: entities_* (the shared people / companies / vehicles / properties / devices / sites registry, if the user turned it on), today (what needs attention across apps), calendar_events (the user's calendar across apps for a date range, with holidays), open_link (otherwise:// deep links). {mode}")
    }

    /// One JSON-RPC message in, its response out (None for notifications).
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned()?;
        if msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(rpc::error(id, rpc::INVALID_REQUEST, "invalid request"));
        }
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        Some(match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = rpc::PROTOCOL_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(rpc::PROTOCOL_VERSIONS[0]);
                rpc::ok(id, json!({"protocolVersion": version, "capabilities": {"tools": {"listChanged": false}}, "serverInfo": {"name": "otherwise-hub", "version": env!("CARGO_PKG_VERSION")}, "instructions": self.instructions()}))
            }
            "ping" => rpc::ok(id, json!({})),
            "tools/list" => rpc::ok(id, json!({"tools": self.tool_list()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call(&name, args) {
                    Ok(result) => rpc::ok(id, result),
                    Err((code, message)) => rpc::error(id, code, &message),
                }
            }
            _ => rpc::error(id, rpc::METHOD_NOT_FOUND, &format!("unknown method {method:?}")),
        })
    }

    /// A `tools/call`: Ok is the MCP result (possibly `isError`), Err a JSON-RPC error.
    pub fn call(&mut self, name: &str, args: Value) -> Result<Value, (i64, String)> {
        if let Some(t) = tools::find(name) {
            if t.write && !self.writes() {
                return Ok(rpc::tool_error("write tools are off: the user can enable them in Otherwise Apps → Hub"));
            }
            return Ok(tools::call(self, name, &args));
        }
        let unknown = || (rpc::INVALID_PARAMS, format!("unknown tool {name:?}"));
        let (app, tool) = split_name(name).ok_or_else(unknown)?;
        let writes_now = self.writes();
        let offered = self.offered_writes();
        let timeout = self.cfg.call_timeout;
        let start_timeout = self.cfg.start_timeout;
        let slot = self.slots.iter_mut().find(|s| s.spec.app == app).ok_or_else(unknown)?;
        let def = slot.tools.iter().find(|t| t.get("name").and_then(Value::as_str) == Some(tool)).cloned();
        if let Some(d) = &def
            && !read_only(d)
        {
            if !(offered && writes_now) {
                return Ok(rpc::tool_error("write tools are off: the user can enable them in Otherwise Apps → Hub"));
            }
            if let Some(why) = &slot.read_only {
                return Ok(rpc::tool_error(&format!("{} runs read-only: its own settings don't allow agent changes ({why})", slot.spec.name)));
            }
        }
        if def.is_none() {
            if slot.tools.is_empty() {
                return Ok(rpc::tool_error(&format!("{} isn't available: {}", slot.spec.name, slot.error.clone().unwrap_or_default())));
            }
            return Err(unknown());
        }
        let params = json!({"name": tool, "arguments": args});
        let first = match slot.server.as_mut() {
            Some(s) => {
                if s.alive() { s.request("tools/call", params.clone(), timeout) } else { Err(CallError::Gone) }
            }
            None => Err(CallError::Gone),
        };
        let res = match first {
            // it died since the last call (or timed out then): one fresh start per call
            Err(CallError::Gone) => {
                slot.server = None;
                let (started, spec, refused) = start_child(slot.spec.clone(), start_timeout);
                if refused.is_some() {
                    // it stopped allowing writes since: read-only from now on (never back)
                    slot.spec = spec;
                    slot.read_only = refused;
                }
                match started {
                    Ok(mut s) => {
                        let r = s.request("tools/call", params, timeout);
                        slot.tools = s.tools.clone();
                        slot.server = Some(s);
                        r
                    }
                    Err(e) => {
                        slot.error = Some(e.to_string());
                        Err(e)
                    }
                }
            }
            r => r,
        };
        match res {
            Ok(v) => Ok(v),
            Err(CallError::Rpc { code, message }) => Err((code, message)),
            Err(e) => {
                if matches!(e, CallError::Timeout) {
                    // a late answer would be taken for the next call's; start fresh next time
                    if let Some(mut s) = slot.server.take() {
                        s.kill();
                    }
                }
                Ok(rpc::tool_error(&format!("{}: {e}", slot.spec.name)))
            }
        }
    }

    /// Line-delimited JSON-RPC over `input` / `output` until `input` ends.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(msg) if msg.is_object() => self.handle(&msg),
                Ok(_) => Some(rpc::error(Value::Null, rpc::INVALID_REQUEST, "batches are not supported")),
                Err(_) => Some(rpc::error(Value::Null, rpc::PARSE_ERROR, "parse error")),
            };
            if let Some(r) = reply {
                writeln!(output, "{r}")?;
                output.flush()?;
            }
        }
        Ok(())
    }
}
