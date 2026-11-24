//! The golden vectors of SPEC §12: every message parses into its type and comes back out with
//! the same JSON (field names are the contract). Vcity cloud's implementation uses the same files.
use otherwise_remote::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn vector(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors").join(name)).unwrap()
}

fn round_trip<T: DeserializeOwned + Serialize>(name: &str) -> T {
    let raw: Value = serde_json::from_str(&vector(name)).unwrap();
    let t: T = serde_json::from_value(raw.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(serde_json::to_value(&t).unwrap(), raw, "{name} round-trips unchanged");
    t
}

#[test]
fn hello_docs_and_doc_hello() {
    let h: Hello = round_trip("hello.json");
    assert_eq!(h.compat(), Compat::Ok);
    assert_eq!(h.me.as_ref().unwrap().role, Some(Role::Owner));
    assert_eq!(h.app("expense").unwrap().protocol, 1);
    let d: DocList = round_trip("docs.json");
    assert_eq!(d.docs[0].role, Role::Editor);
    let dh: DocHello = round_trip("doc-hello.json");
    assert!(!dh.writable);
    assert_eq!(Status::from_doc(&dh), Status::ReadOnly { reason: "role".into() });
    assert_eq!(dh.seats.unwrap().over_since.as_deref(), Some("2026-11-20"));
}

#[test]
fn ops_undo_and_errors() {
    let r: OpRequest = round_trip("op-request.json");
    assert_eq!(r.base_revision, Some(1234));
    let res: OpResult = round_trip("op-result.json");
    assert_eq!(res.change_id.as_deref(), Some("c41"));
    let u: UndoRequest = round_trip("undo-request.json");
    assert_eq!(u.change_id.as_deref(), Some("c41"));
    let v: Value = serde_json::from_str(&vector("error-conflict.json")).unwrap();
    let e: ApiError = serde_json::from_value(v["error"].clone()).unwrap();
    assert_eq!((e.code.as_str(), e.status()), ("conflict", 409));
    assert_eq!(e.extra["revision"], 1236);
    assert_eq!(e.envelope(), v);
}

#[test]
fn unknown_fields_are_ignored_and_newer_protocols_detected() {
    let h: Hello = serde_json::from_str(r#"{"server":"otherwise","protocols":{"min":2,"max":3},"future":true}"#).unwrap();
    assert_eq!(h.compat(), Compat::ClientTooOld);
    let h: Hello = serde_json::from_str(r#"{"server":"nginx","protocols":{"min":1,"max":1}}"#).unwrap();
    assert_eq!(h.compat(), Compat::NotOtherwise);
}

#[test]
fn events_stream() {
    let mut r = sse::Reader::new(std::io::Cursor::new(vector("events.sse")));
    let mut events = Vec::new();
    while let Some(m) = r.next_message().unwrap() {
        events.push(serde_json::from_str::<Event>(&m.data).unwrap());
    }
    assert_eq!(events[0], Event::Changed { revision: 1235, tables: vec!["receipts".into()], actor: Some("ana@example.com@macbookpro".into()) });
    assert_eq!(events[1], Event::Unknown, "a newer kind is skipped, not an error");
    assert_eq!(events[2], Event::Bye { reason: "rotate".into() });
    assert_eq!(r.last_id(), Some("1235"));
}

#[test]
fn status_codes() {
    for (code, status) in [("unauthorized", 401), ("forbidden", 403), ("people_only", 403), ("not_found", 404), ("licence", 402), ("read_only", 409), ("unknown_op", 400), ("unavailable", 503), ("failed", 500)] {
        assert_eq!(status_for(code), status, "{code}");
    }
    assert!(Role::Owner > Role::Editor && Role::Editor.can_write() && !Role::Viewer.can_write());
    assert_eq!(Role::parse("agent"), Some(Role::Agent));
    let (a, b) = (new_key(), new_key());
    assert_eq!(a.len(), 32);
    assert_ne!(a, b);
}
