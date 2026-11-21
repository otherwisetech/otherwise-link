//! The hub's own tools.
use crate::hub::Hub;
use crate::rpc::{tool_error, tool_result};
use otherwise_link::entity::{Entity, Kind};
use otherwise_link::link::Link;
use otherwise_link::open::Opened;
use otherwise_link::registry::{Matched, Registry};
use otherwise_link::today;
use serde_json::{Value, json};
use uuid::Uuid;

/// The app id the registry records for changes made through the hub.
pub const WRITER: &str = "mcp";

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub write: bool,
    pub schema: fn() -> Value,
}

fn kinds() -> Value {
    json!(Kind::ALL.iter().map(|k| k.key()).collect::<Vec<_>>())
}

pub static TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "entities_search",
        description: "Search the shared entity registry (people, companies, vehicles, properties, devices, sites, digital assets known to the user's Otherwise apps). Matches names, aliases, tax ids, plates, MACs, hostnames, domains; accents and case are ignored. Returns cards with their per-app references.",
        write: false,
        schema: || json!({"type": "object", "properties": {"query": {"type": "string", "description": "Words that must all appear; empty lists everything"}, "kind": {"type": "string", "enum": kinds()}, "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}}}),
    },
    ToolDef {
        name: "entities_get",
        description: "One entity card by its id (merged-away ids resolve to the survivor), or by an app's own id (app + ref). Includes which apps reference it and an otherwise:// link into each.",
        write: false,
        schema: || json!({"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}, "app": {"type": "string"}, "ref": {"type": "string"}}}),
    },
    ToolDef {
        name: "today",
        description: "What needs the user's attention across their Otherwise apps: deadlines (due), alerts, things to review and notices, merged from every app's Today feed, soonest first. Each event has the app, title, when, severity and an otherwise:// link.",
        write: false,
        schema: || json!({"type": "object", "properties": {"days": {"type": "integer", "minimum": 0, "description": "Only events up to this many days ahead (overdue and undated ones included)"}, "app": {"type": "string", "description": "Only this app's events"}}}),
    },
    ToolDef {
        name: "open_link",
        description: "Open an otherwise:// deep link (from a card or a Today event) in its app on the user's screen.",
        write: false,
        schema: || json!({"type": "object", "properties": {"url": {"type": "string", "description": "otherwise://<app>/<path>"}}, "required": ["url"]}),
    },
    ToolDef {
        name: "entities_upsert",
        description: "Create or update an entity card in the shared registry. An existing card is found by id, refs or natural keys (tax id, plate, MAC…); set fields win, lists are merged, nothing is removed. Card JSON: {kind, name, aliases?, refs?, tags?, notes?, <kind>: {...}} as in otherwise-link's docs/SPEC.md.",
        write: true,
        schema: || json!({"type": "object", "properties": {"card": {"type": "object", "description": "The entity card", "properties": {"kind": {"type": "string", "enum": kinds()}, "name": {"type": "string"}}, "required": ["kind", "name"]}}, "required": ["card"]}),
    },
    ToolDef {
        name: "entities_link",
        description: "Record that app <app> knows entity <id> as <ref> (sets refs[app]).",
        write: true,
        schema: || json!({"type": "object", "properties": {"id": {"type": "string", "format": "uuid"}, "app": {"type": "string"}, "ref": {"type": "string"}}, "required": ["id", "app", "ref"]}),
    },
];

pub fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

pub fn own(writes: bool) -> Vec<Value> {
    TOOLS
        .iter()
        .filter(|t| writes || !t.write)
        .map(|t| {
            let read_only = !t.write && t.name != "open_link";
            json!({"name": t.name, "description": t.description, "inputSchema": (t.schema)(), "annotations": {"readOnlyHint": read_only, "destructiveHint": false}})
        })
        .collect()
}

fn s<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str).filter(|v| !v.is_empty())
}

fn with_links(reg: &Registry, e: &Entity) -> Value {
    let refs = reg.referencing_apps(e.id).unwrap_or_default();
    let refs: Vec<Value> = refs
        .into_iter()
        .map(|(app, r)| {
            let link = otherwise_link::link::valid_app_id(&app).then(|| Link::new(&app).segment("entity").segment(e.id.to_string()).to_url());
            json!({"app": app, "appName": otherwise_link::apps::display_name(&app), "ref": r, "link": link})
        })
        .collect();
    json!({"entity": e, "references": refs})
}

pub fn call(hub: &Hub, name: &str, args: &Value) -> Value {
    match run(hub, name, args) {
        Ok(v) => tool_result(v),
        Err(e) => tool_error(&e),
    }
}

fn registry(hub: &Hub) -> Result<Registry, String> {
    Registry::open_shared(&hub.cfg.root).map_err(|e| e.to_string())
}

fn uuid_arg(args: &Value) -> Result<Uuid, String> {
    let id = s(args, "id").ok_or("id is required")?;
    Uuid::parse_str(id).map_err(|_| format!("{id:?} is not a UUID"))
}

fn run(hub: &Hub, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "entities_search" => {
            let reg = registry(hub)?;
            let kind = match s(args, "kind") {
                Some(k) => Some(Kind::parse(k).ok_or(format!("unknown kind {k:?}"))?),
                None => None,
            };
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 100) as usize;
            let found = reg.search(s(args, "query").unwrap_or(""), kind, limit).map_err(|e| e.to_string())?;
            Ok(json!({"entities": found.iter().map(|e| with_links(&reg, e)).collect::<Vec<_>>()}))
        }
        "entities_get" => {
            let reg = registry(hub)?;
            let e = if s(args, "id").is_some() {
                reg.get(uuid_arg(args)?).map_err(|e| e.to_string())?
            } else {
                let (Some(app), Some(r)) = (s(args, "app"), s(args, "ref")) else { return Err("give id, or app and ref".into()) };
                reg.by_ref(app, r).map_err(|e| e.to_string())?
            };
            let e = e.ok_or("no such entity")?;
            Ok(with_links(&reg, &e))
        }
        "entities_upsert" => {
            let mut reg = registry(hub)?;
            let card = Entity::from_value(args.get("card").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
            let up = reg.upsert(WRITER, card).map_err(|e| e.to_string())?;
            let matched = match up.matched {
                Matched::Created => "created",
                Matched::Id => "id",
                Matched::OwnRef | Matched::OtherRef => "ref",
                Matched::Key => "key",
            };
            Ok(json!({"id": up.id, "matched": matched, "entity": up.entity}))
        }
        "entities_link" => {
            let mut reg = registry(hub)?;
            let id = uuid_arg(args)?;
            let (Some(app), Some(r)) = (s(args, "app"), s(args, "ref")) else { return Err("app and ref are required".into()) };
            let e = reg.link(app, id, r).map_err(|e| e.to_string())?;
            Ok(with_links(&reg, &e))
        }
        "today" => {
            let now = chrono::Utc::now();
            let merged = today::read(&hub.cfg.root, now);
            let horizon = args.get("days").and_then(Value::as_i64).map(|d| chrono::Local::now().date_naive() + chrono::Duration::days(d));
            let app = s(args, "app");
            let events: Vec<&today::Event> = merged
                .events
                .iter()
                .filter(|e| app.is_none_or(|a| e.app == a))
                .filter(|e| match (horizon, e.when_parsed()) {
                    (Some(h), Some(w)) => w.day(&chrono::Local) <= h,
                    _ => true,
                })
                .collect();
            let errors: Vec<Value> = merged.errors.iter().map(|(f, e)| json!({"file": f, "error": e})).collect();
            Ok(json!({"today": chrono::Local::now().date_naive().to_string(), "events": events, "errors": errors}))
        }
        "open_link" => {
            let url = s(args, "url").ok_or("url is required")?;
            let link = Link::parse(url).map_err(|e| e.to_string())?;
            match (hub.cfg.opener)(&link) {
                Ok(Opened::App) => Ok(json!({"opened": link.to_url(), "by": link.app})),
                Ok(Opened::Hub) => Ok(json!({"opened": link.to_url(), "by": otherwise_link::apps::HUB_ID})),
                Err(e) => Err(format!("couldn't open {url}: {e}")),
            }
        }
        _ => Err(format!("unknown tool {name:?}")),
    }
}
