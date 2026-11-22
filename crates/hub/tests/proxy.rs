//! Hub tests, with this same binary as the fake app MCP server (`FAKE_MCP_CHILD=<mode>`).
use otherwise_link::Root;
use otherwise_link::link::Link;
use otherwise_link::open::Opened;
use otherwise_link::settings::HubSettings;
use otherwise_link::today::{self, Event, EventKind};
use otherwise_mcp_hub::child::ChildSpec;
use otherwise_mcp_hub::hub::{Config, Hub, children_for, split_name};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn fake_child(mode: &str) {
    let writes = std::env::args().any(|a| a == "--allow-writes");
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    // noise a real server might print: a non-JSON line and a log notification
    writeln!(out, "starting up…").unwrap();
    writeln!(out, "{}", json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "hi"}})).unwrap();
    out.flush().unwrap();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let msg: Value = serde_json::from_str(&line).unwrap();
        let Some(id) = msg.get("id").cloned() else { continue };
        if msg.get("method").is_none() {
            continue; // the hub answering our ping
        }
        let method = msg["method"].as_str().unwrap();
        if mode == "hang" {
            continue;
        }
        let result = match method {
            "initialize" => json!({"protocolVersion": msg["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}}),
            "tools/list" if msg["params"].get("cursor").is_none() => {
                // ask the hub something first: it must answer and keep going
                writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": "c1", "method": "ping"})).unwrap();
                json!({"tools": [{"name": "search", "description": "find things", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}}], "nextCursor": "p2"})
            }
            "tools/list" => json!({"tools": [{"name": "set", "description": "change things", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": false}}, {"name": "unannotated", "description": "no hints", "inputSchema": {"type": "object"}}]}),
            "tools/call" => {
                let args = &msg["params"]["arguments"];
                if args.get("crash").is_some() {
                    std::process::exit(3);
                }
                if let Some(ms) = args.get("sleep").and_then(Value::as_u64) {
                    std::thread::sleep(Duration::from_millis(ms));
                }
                if args.get("fail").is_some() {
                    writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "bad args"}})).unwrap();
                    out.flush().unwrap();
                    continue;
                }
                let v = json!({"tool": msg["params"]["name"], "args": args, "writes": writes, "pid": std::process::id()});
                json!({"content": [{"type": "text", "text": v.to_string()}], "structuredContent": v})
            }
            _ => {
                writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "nope"}})).unwrap();
                out.flush().unwrap();
                continue;
            }
        };
        writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": id, "result": result})).unwrap();
        out.flush().unwrap();
    }
}

fn spec(app: &str, mode: &str, writes: bool) -> ChildSpec {
    let mut args = vec![format!("--mode={mode}")];
    if writes {
        args.push("--allow-writes".into());
    }
    ChildSpec { app: app.into(), name: format!("Fake {app}"), program: std::env::current_exe().unwrap(), args }
}

struct Env {
    _dir: tempfile::TempDir,
    root: Root,
    opened: Arc<Mutex<Vec<String>>>,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    Env { root: Root::at(dir.path()), _dir: dir, opened: Arc::new(Mutex::new(vec![])) }
}

fn hub(e: &Env, children: Vec<ChildSpec>, writes: Option<bool>) -> Hub {
    let mut cfg = Config::new(e.root.clone(), children);
    cfg.writes = writes;
    cfg.start_timeout = Duration::from_secs(5);
    cfg.call_timeout = Duration::from_secs(5);
    let opened = e.opened.clone();
    cfg.opener = Arc::new(move |l: &Link| {
        opened.lock().unwrap().push(l.to_url());
        Ok(if l.app == "vigia" { Opened::App } else { Opened::Hub })
    });
    Hub::start(cfg)
}

fn rpc(h: &mut Hub, id: i64, method: &str, params: Value) -> Value {
    h.handle(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap()
}

fn call(h: &mut Hub, name: &str, args: Value) -> Value {
    rpc(h, 9, "tools/call", json!({"name": name, "arguments": args}))
}

fn names(h: &mut Hub) -> Vec<String> {
    let r = rpc(h, 1, "tools/list", json!({}));
    r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

fn read_only_by_default() {
    let e = env();
    let mut h = hub(&e, vec![spec("vigia", "ok", false)], None);
    let n = names(&mut h);
    assert_eq!(n, ["entities_search", "entities_get", "today", "calendar_events", "open_link", "vigia.search"], "write tools and unannotated child tools are hidden");
    let r = call(&mut h, "vigia.search", json!({"q": "soat"}));
    assert_eq!(r["result"]["structuredContent"]["tool"], "search");
    assert_eq!(r["result"]["structuredContent"]["args"]["q"], "soat");
    assert_eq!(r["result"]["structuredContent"]["writes"], false);
    // both separators route
    assert_eq!(call(&mut h, "vigia__search", json!({}))["result"]["structuredContent"]["tool"], "search");
    assert_eq!(call(&mut h, "vigia.set", json!({}))["result"]["isError"], true);
    assert_eq!(call(&mut h, "vigia.nope", json!({}))["error"]["code"], -32602);
    assert_eq!(call(&mut h, "other.search", json!({}))["error"]["code"], -32602);
    assert_eq!(call(&mut h, "entities_upsert", json!({"card": {"kind": "person", "name": "A"}}))["result"]["isError"], true);
    // the registry is off until the user opts in
    let r = call(&mut h, "entities_search", json!({"query": "x"}));
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("off"));
    // child errors pass through as JSON-RPC errors
    assert_eq!(call(&mut h, "vigia.search", json!({"fail": true}))["error"]["message"], "bad args");
}

fn writes_when_enabled() {
    let e = env();
    HubSettings::update(&e.root, |s| {
        s.entities = true;
        s.mcp_writes = true;
    })
    .unwrap();
    let mut h = hub(&e, vec![spec("vigia", "ok", true)], None);
    let n = names(&mut h);
    for t in ["entities_upsert", "entities_link", "vigia.set", "vigia.unannotated"] {
        assert!(n.contains(&t.to_string()), "{t} in {n:?}");
    }
    let r = call(&mut h, "vigia.set", json!({"x": 1}));
    assert_eq!(r["result"]["structuredContent"]["writes"], true);

    let up = call(&mut h, "entities_upsert", json!({"card": {"kind": "company", "name": "ACME", "company": {"taxIds": [{"country": "CO", "scheme": "NIT", "value": "800.197.268-4"}]}, "refs": {"vigia": "e1"}}}));
    let up = &up["result"]["structuredContent"];
    assert_eq!(up["matched"], "created");
    let id = up["id"].as_str().unwrap().to_string();
    let again = call(&mut h, "entities_upsert", json!({"card": {"kind": "company", "name": "Acme SAS", "company": {"taxIds": [{"country": "CO", "scheme": "NIT", "value": "8001972684"}]}}}));
    assert_eq!(again["result"]["structuredContent"]["matched"], "key");
    let linked = call(&mut h, "entities_link", json!({"id": id, "app": "nodo", "ref": "d9"}));
    assert_eq!(linked["result"]["structuredContent"]["entity"]["refs"]["nodo"], "d9");

    let found = call(&mut h, "entities_search", json!({"query": "acme", "kind": "company"}));
    let list = found["result"]["structuredContent"]["entities"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    let refs = list[0]["references"].as_array().unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0]["link"], format!("otherwise://nodo/entity/{id}"));
    let got = call(&mut h, "entities_get", json!({"app": "vigia", "ref": "e1"}));
    assert_eq!(got["result"]["structuredContent"]["entity"]["id"], id.as_str());
    assert_eq!(call(&mut h, "entities_get", json!({"id": "nope"}))["result"]["isError"], true);
    assert_eq!(call(&mut h, "entities_search", json!({"kind": "planet"}))["result"]["isError"], true);
    assert_eq!(call(&mut h, "entities_upsert", json!({"card": {"kind": "person", "name": " "}}))["result"]["isError"], true);

    // turning writes off applies to the next call, without a restart
    HubSettings::update(&e.root, |s| s.mcp_writes = false).unwrap();
    assert_eq!(call(&mut h, "vigia.set", json!({}))["result"]["isError"], true);
    assert_eq!(call(&mut h, "entities_link", json!({"id": id, "app": "x", "ref": "1"}))["result"]["isError"], true);
    assert!(!names(&mut h).contains(&"vigia.set".to_string()));
}

fn crashed_child_restarts() {
    let e = env();
    let mut h = hub(&e, vec![spec("nodo", "ok", false)], Some(false));
    let pid = call(&mut h, "nodo.search", json!({}))["result"]["structuredContent"]["pid"].as_u64().unwrap();
    let r = call(&mut h, "nodo.search", json!({"crash": true}));
    assert_eq!(r["result"]["isError"], true, "{r}");
    let r = call(&mut h, "nodo.search", json!({}));
    let pid2 = r["result"]["structuredContent"]["pid"].as_u64().unwrap();
    assert_ne!(pid, pid2, "a fresh process answered");
}

fn slow_and_broken_children_are_skipped() {
    let e = env();
    let missing = ChildSpec { app: "expense".into(), name: "expense".into(), program: "/nonexistent/expense-cli".into(), args: vec!["mcp".into()] };
    let mut cfg = Config::new(e.root.clone(), vec![spec("vigia", "hang", false), missing, spec("nodo", "ok", false)]);
    cfg.writes = Some(false);
    cfg.start_timeout = Duration::from_millis(800);
    cfg.call_timeout = Duration::from_millis(500);
    let started = std::time::Instant::now();
    let mut h = Hub::start(cfg);
    assert!(started.elapsed() < Duration::from_secs(4), "children start in parallel");
    let st = h.status();
    assert!(st[0].tools.is_err() && st[1].tools.is_err() && st[2].tools == Ok(3), "{st:?}");
    let n = names(&mut h);
    assert!(n.contains(&"nodo.search".to_string()) && !n.iter().any(|t| t.starts_with("vigia.") || t.starts_with("expense.")));
    assert_eq!(call(&mut h, "vigia.search", json!({}))["result"]["isError"], true);
    // a call that times out gives an error and a fresh child next time
    let r = call(&mut h, "nodo.search", json!({"sleep": 1500}));
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("too long"));
    assert_eq!(call(&mut h, "nodo.search", json!({"n": 2}))["result"]["structuredContent"]["args"]["n"], 2);
    let init = rpc(&mut h, 1, "initialize", json!({"protocolVersion": "2025-06-18"}));
    let text = init["result"]["instructions"].as_str().unwrap();
    assert!(text.contains("nodo.*") && !text.contains("vigia.*"), "{text}");
}

fn today_and_open_link() {
    let e = env();
    today::publish(&e.root, "vigia", &[Event::new("soat", EventKind::Due, "SOAT").when("2099-01-01").link(&Link::new("vigia").segment("item").segment("1")), Event::new("now", EventKind::Alert, "Now")]).unwrap();
    today::publish(&e.root, "nodo", &[Event::new("down", EventKind::Alert, "NAS down")]).unwrap();
    let mut h = hub(&e, vec![], None);
    let all = call(&mut h, "today", json!({}));
    assert_eq!(all["result"]["structuredContent"]["events"].as_array().unwrap().len(), 3);
    let soon = call(&mut h, "today", json!({"days": 7}));
    assert_eq!(soon["result"]["structuredContent"]["events"].as_array().unwrap().len(), 2, "far-off events are cut, undated kept");
    let vigia = call(&mut h, "today", json!({"app": "vigia"}));
    assert_eq!(vigia["result"]["structuredContent"]["events"][0]["link"], "otherwise://vigia/item/1");

    let r = call(&mut h, "open_link", json!({"url": "otherwise://vigia/item/1"}));
    assert_eq!(r["result"]["structuredContent"]["by"], "vigia");
    let r = call(&mut h, "open_link", json!({"url": "otherwise://rama/repo"}));
    assert_eq!(r["result"]["structuredContent"]["by"], "otherwise-apps");
    assert_eq!(call(&mut h, "open_link", json!({"url": "https://evil.example"}))["result"]["isError"], true);
    assert_eq!(*e.opened.lock().unwrap(), ["otherwise://vigia/item/1", "otherwise://rama/repo"]);

    use otherwise_link::events::{self as cal, Kind};
    cal::publish(&e.root, "vigia", &[cal::Event::new("soat", Kind::Due, "SOAT", "2026-12-01").link(&Link::new("vigia").segment("item").segment("1"))], None).unwrap();
    cal::publish(&e.root, "expense", &[cal::Event::new("rent", Kind::Payment, "Rent 4.500.000", "2026-11-05").rrule("FREQ=MONTHLY").busy_only(true).body("secret")], None).unwrap();
    let r = call(&mut h, "calendar_events", json!({"from": "2026-11-01", "to": "2026-12-31", "holidays": ["co"]}));
    let ev = r["result"]["structuredContent"]["events"].as_array().unwrap().clone();
    let got: Vec<(String, String)> = ev.iter().map(|x| (x["firstDay"].as_str().unwrap().to_string(), x["app"].as_str().unwrap().to_string())).collect();
    assert_eq!(got.len(), 2 + 1 + 4, "{got:?}");
    let rent = ev.iter().find(|x| x["app"] == "expense").unwrap();
    assert_eq!((rent["title"].clone(), rent["body"].clone()), (json!("busy"), Value::Null), "busy-only events are redacted");
    assert!(!r.to_string().contains("4.500.000"));
    let only = call(&mut h, "calendar_events", json!({"from": "2026-11-01", "to": "2026-12-31", "apps": ["vigia"]}));
    assert_eq!(only["result"]["structuredContent"]["events"][0]["link"], "otherwise://vigia/item/1");
    assert_eq!(call(&mut h, "calendar_events", json!({"from": "2026-01-01", "to": "2028-01-01"}))["result"]["isError"], true);
    assert_eq!(call(&mut h, "calendar_events", json!({"from": "nope"}))["result"]["isError"], true);
}

fn protocol() {
    let e = env();
    let mut h = hub(&e, vec![], Some(false));
    let r = rpc(&mut h, 1, "initialize", json!({"protocolVersion": "2025-03-26", "capabilities": {}}));
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(r["result"]["serverInfo"]["name"], "otherwise-hub");
    assert!(r["result"]["instructions"].as_str().unwrap().contains("Read-only"));
    assert_eq!(rpc(&mut h, 2, "initialize", json!({"protocolVersion": "1999-01-01"}))["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(rpc(&mut h, 3, "ping", json!({}))["result"], json!({}));
    assert_eq!(rpc(&mut h, 4, "resources/list", json!({}))["error"]["code"], -32601);
    assert!(h.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
    assert_eq!(h.handle(&json!({"id": 5, "method": "ping"})).unwrap()["error"]["code"], -32600);
    assert_eq!(split_name("a.b.c"), Some(("a", "b.c")));
    assert_eq!(split_name("nodots"), None);
    assert_eq!(split_name(".x"), None);

    // the stdio loop end to end
    let input = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}).to_string(),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        String::new(),
        "{not json".to_string(),
        "[1, 2]".to_string(),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string(),
    ]
    .join("\n");
    let mut out = Vec::new();
    h.serve(input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0]["id"], 1);
    assert_eq!(lines[1]["error"]["code"], -32700);
    assert_eq!(lines[2]["error"]["code"], -32600);
    assert_eq!(lines[3]["result"]["tools"].as_array().unwrap().len(), 5);
}

fn discovery_to_children() {
    use otherwise_link::apps::{Installed, known};
    let inst = |id: &str, cli: Option<&str>| Installed { id: id.into(), path: None, executable: None, cli: cli.map(Into::into), version: None, bundle_id: None };
    let found = vec![
        (known("expense").unwrap().spec(), inst("expense", Some("/A/expense.app/Contents/MacOS/expense-cli"))),
        (known("vigia").unwrap().spec(), inst("vigia", Some("/usr/local/bin/vigia"))),
        (known("nodo").unwrap().spec(), inst("nodo", None)),
        (known("rama").unwrap().spec(), inst("rama", Some("/x/rama"))),
        (known("otherwise-apps").unwrap().spec(), inst("otherwise-apps", Some("/x/otherwise-apps"))),
    ];
    let mut s = HubSettings::default();
    let c = children_for(&s, &found);
    let got: Vec<(&str, Vec<&str>)> = c.iter().map(|c| (c.app.as_str(), c.args.iter().map(String::as_str).collect())).collect();
    assert_eq!(got, [("expense", vec!["mcp"]), ("vigia", vec!["mcp"])], "no CLI, no MCP capability, or the hub itself: skipped");
    s.mcp_writes = true;
    s.mcp_disabled = vec!["vigia".into()];
    let c = children_for(&s, &found);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].args, ["mcp", "--allow-writes"]);
}

fn main() {
    if let Some(mode) = std::env::args().find_map(|a| a.strip_prefix("--mode=").map(str::to_string)) {
        fake_child(&mode);
        return;
    }
    let tests: &[(&str, fn())] = &[
        ("read_only_by_default", read_only_by_default),
        ("writes_when_enabled", writes_when_enabled),
        ("crashed_child_restarts", crashed_child_restarts),
        ("slow_and_broken_children_are_skipped", slow_and_broken_children_are_skipped),
        ("today_and_open_link", today_and_open_link),
        ("protocol", protocol),
        ("discovery_to_children", discovery_to_children),
    ];
    let filter: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let mut failed = 0;
    for (name, f) in tests {
        if !filter.is_empty() && !filter.iter().any(|x| name.contains(x.as_str())) {
            continue;
        }
        match std::panic::catch_unwind(f) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        println!("test result: FAILED. {failed} failed");
        std::process::exit(1);
    }
    println!("test result: ok.");
}
