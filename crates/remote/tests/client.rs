//! The client against a local fake server (never the network).
use otherwise_remote::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// Answers each request with `answer(method, path, headers)`: (status, extra headers, body).
type Answer = dyn Fn(&Seen) -> (u16, Vec<(&'static str, String)>, Vec<u8>) + Send + Sync;

fn serve(answer: Arc<Answer>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut conn) = conn else { continue };
            let (answer, seen) = (answer.clone(), s2.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let mut headers = Vec::new();
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    let (k, v) = h.split_once(':').unwrap();
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap();
                    }
                    headers.push((k.to_ascii_lowercase(), v.trim().to_string()));
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let req = Seen { method, path, headers, body: String::from_utf8(body).unwrap() };
                let (status, extra, body) = answer(&req);
                seen.lock().unwrap().push(req);
                let mut head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                for (k, v) in extra {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                let _ = conn.write_all(head.as_bytes());
                let _ = conn.write_all(&body);
            });
        }
    });
    (url, seen)
}

fn json(v: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&v).unwrap()
}

#[test]
fn hello_docs_snapshot_and_ops() {
    let (url, seen) = serve(Arc::new(|r: &Seen| match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/v1/hello") => (200, vec![], json(serde_json::json!({"server": "otherwise", "protocols": {"min": 1, "max": 1}, "host": "studio", "apps": [{"id": "expense", "protocol": 1}]}))),
        ("GET", "/v1/apps/expense/docs") => (200, vec![], json(serde_json::json!({"docs": [{"id": "acme", "name": "Acme", "role": "editor"}]}))),
        ("GET", "/v1/apps/expense/docs/acme/snapshot") => {
            if r.headers.iter().any(|(k, v)| k == "if-none-match" && v == "\"7\"") {
                (304, vec![], vec![])
            } else {
                (200, vec![("ETag", "\"7\"".into()), ("X-Otherwise-Revision", "7".into())], b"SQLite format 3\0fake".to_vec())
            }
        }
        ("POST", "/v1/apps/expense/docs/acme/ops") => {
            let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
            if v["op"] == "close_month" {
                (403, vec![], json(serde_json::json!({"ok": false, "error": {"code": "people_only", "message": "people only", "op": "close_month"}})))
            } else {
                (200, vec![], json(serde_json::json!({"ok": true, "result": {"change": "c1"}, "revision": 8, "changeId": "c1"})))
            }
        }
        ("GET", "/v1/apps/expense/docs/secret/hello") => (404, vec![], json(serde_json::json!({"ok": false, "error": {"code": "not_found", "message": "no such document"}}))),
        _ => (500, vec![], b"plain text failure".to_vec()),
    }));
    let c = Client::new(&url);
    let h = c.hello().unwrap();
    assert_eq!((h.host.as_str(), h.compat()), ("studio", Compat::Ok));
    assert_eq!(c.docs("expense").unwrap()[0].id, "acme");

    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("replica/expense.db");
    let s = c.snapshot("expense", "acme", None, &dest).unwrap();
    assert_eq!(s, Snapshot::Saved { revision: 7, etag: "7".into(), bytes: 20 });
    assert!(std::fs::read(&dest).unwrap().starts_with(b"SQLite format 3"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o600, "replicas are private");
    }
    assert_eq!(c.snapshot("expense", "acme", Some("7"), &dest).unwrap(), Snapshot::NotModified);
    assert!(std::fs::read_dir(dest.parent().unwrap()).unwrap().count() == 1, "no temp file left");

    let key = new_key();
    let r = c.op("expense", "acme", &OpRequest { op: "set_receipt".into(), args: serde_json::json!({"id": "r1"}), base_revision: Some(7), idempotency_key: key.clone(), command: None }).unwrap();
    assert_eq!((r.revision, r.change_id.as_deref()), (8, Some("c1")));
    let sent = seen.lock().unwrap().iter().rev().find(|s| s.path.ends_with("/ops")).cloned().unwrap();
    let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!((body["idempotencyKey"].as_str(), body["baseRevision"].as_u64()), (Some(key.as_str()), Some(7)));
    assert!(!sent.headers.iter().any(|(k, _)| k == "authorization" || k.starts_with("tailscale-")), "the client never claims an identity");

    match c.op("expense", "acme", &OpRequest { op: "close_month".into(), args: serde_json::json!({}), base_revision: None, idempotency_key: new_key(), command: None }) {
        Err(ClientError::Api { status: 403, error }) => assert_eq!((error.code.as_str(), error.extra["op"].as_str()), ("people_only", Some("close_month"))),
        other => panic!("{other:?}"),
    }
    let e = c.doc_hello("expense", "secret").unwrap_err();
    assert_eq!((e.code(), Status::from_error(&e, None)), ("not_found", Status::NoAccess));
    // a non-JSON error still has a code
    let e = c.docs("other").unwrap_err();
    assert_eq!(e.code(), "failed");
    // ids never build other paths
    assert!(matches!(c.doc_hello("expense", "../hello"), Err(ClientError::Protocol(_))));
}

#[test]
fn events_and_last_event_id() {
    let (url, seen) = serve(Arc::new(|r: &Seen| {
        if r.path.ends_with("/events") {
            let body = format!("{}{}{}", sse::KEEPALIVE, sse::format("changed", Some("9"), r#"{"event":"changed","revision":9}"#), sse::format("bye", Some("9"), r#"{"event":"bye","reason":"rotate"}"#));
            (200, vec![("Content-Type", "text/event-stream".into())], body.into_bytes())
        } else {
            (401, vec![], json(serde_json::json!({"ok": false, "error": {"code": "unauthorized", "message": "no tailscale identity"}})))
        }
    }));
    let c = Client::new(&url);
    let mut ev = c.events("expense", "acme", Some("8")).unwrap();
    assert_eq!(ev.next(), Some(Event::Changed { revision: 9, tables: vec![], actor: None }));
    assert_eq!(ev.next(), Some(Event::Bye { reason: "rotate".into() }));
    assert_eq!(ev.next(), None);
    assert_eq!(ev.last_id(), Some("9"));
    assert!(seen.lock().unwrap()[0].headers.iter().any(|(k, v)| k == "last-event-id" && v == "8"));
    let e = c.hello().unwrap_err();
    assert_eq!(Status::from_error(&e, None), Status::NoAccess);
}

#[test]
fn offline_and_discovery() {
    // nothing listens there
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let c = Client::with_timeout(&format!("http://127.0.0.1:{port}"), std::time::Duration::from_secs(2));
    let e = c.hello().unwrap_err();
    assert!(e.is_offline());
    assert_eq!(Status::from_error(&e, Some(5)), Status::Offline { since: Some(5) });

    let (url, _) = serve(Arc::new(|_: &Seen| (200, vec![], json(serde_json::json!({"server": "otherwise", "protocols": {"min": 1, "max": 1}, "host": "studio"})))));
    let (other, _) = serve(Arc::new(|_: &Seen| (200, vec![], b"<html>".to_vec())));
    let peers = vec![
        discover::Peer { host_name: "studio".into(), dns_name: "studio.t.ts.net".into(), os: "macOS".into(), online: true },
        discover::Peer { host_name: "nas".into(), dns_name: "nas.t.ts.net".into(), os: "linux".into(), online: true },
        discover::Peer { host_name: "off".into(), dns_name: "off.t.ts.net".into(), os: "linux".into(), online: false },
    ];
    let found = discover::probe_with(&peers, std::time::Duration::from_secs(2), |p| if p.host_name == "studio" { url.clone() } else { other.clone() });
    assert_eq!(found.iter().map(|(p, _)| p.host_name.as_str()).collect::<Vec<_>>(), ["studio"]);
}
