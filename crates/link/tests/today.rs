use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use otherwise_link::Root;
use otherwise_link::today::{self, Event, EventKind, Severity, When};
use otherwise_link::Link;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn bogota() -> FixedOffset {
    FixedOffset::west_opt(5 * 3600).unwrap()
}

#[test]
fn publish_and_merge() {
    let t = tempfile::tempdir().unwrap();
    let root = Root::at(t.path());
    let tz = bogota();
    today::publish(
        &root,
        "vigia",
        &[
            Event::new("soat", EventKind::Due, "SOAT ABC123").when("2026-11-30").severity(Severity::High).link(&Link::new("vigia").segment("item").segment("42")),
            Event::new("old", EventKind::Due, "expired yesterday").when("2026-11-01").expires("2026-11-20"),
            Event::new("undated", EventKind::Info, "Backup suggested"),
        ],
    )
    .unwrap();
    today::publish(
        &root,
        "nodo",
        &[
            Event::new("down", EventKind::Alert, "NAS is down").when("2026-11-21T13:00:00Z").severity(Severity::Critical),
            Event::new("same-day", EventKind::Info, "low thing").when("2026-11-30T15:00:00-05:00").severity(Severity::Low),
            Event::new("tie", EventKind::Review, "same moment, higher").when("2026-11-30").severity(Severity::Critical),
        ],
    )
    .unwrap();
    let now = at("2026-11-21T14:00:00Z");
    let m = today::read_in(&root, now, &tz);
    assert!(m.errors.is_empty(), "{:?}", m.errors);
    let order: Vec<(&str, &str)> = m.events.iter().map(|e| (e.app.as_str(), e.id.as_str())).collect();
    assert_eq!(order, [("nodo", "down"), ("nodo", "tie"), ("vigia", "soat"), ("nodo", "same-day"), ("vigia", "undated")]);
    assert_eq!(m.updated.len(), 2);
    assert_eq!(m.events[2].link.as_deref(), Some("otherwise://vigia/item/42"));

    let days = today::by_day(&m.events, &tz);
    let keys: Vec<Option<NaiveDate>> = days.iter().map(|(d, _)| *d).collect();
    assert_eq!(keys, [NaiveDate::from_ymd_opt(2026, 11, 21), NaiveDate::from_ymd_opt(2026, 11, 30), None]);
    assert_eq!(days[1].1.len(), 3);
    let apps: Vec<String> = today::by_app(&m.events).into_iter().map(|(a, _)| a).collect();
    assert_eq!(apps, ["nodo", "vigia"]);

    // republishing replaces; clearing removes
    today::publish(&root, "nodo", &[]).unwrap();
    assert_eq!(today::read_in(&root, now, &tz).events.len(), 2);
    today::clear(&root, "nodo").unwrap();
    today::clear(&root, "nodo").unwrap();
    assert!(!root.today_dir().join("nodo.json").exists());
}

#[test]
fn expiry_uses_the_end_of_the_local_day() {
    let tz = bogota();
    let e = Event::new("x", EventKind::Due, "x").expires("2026-11-20");
    // 2026-11-20 23:59 in Bogotá is 2026-11-21 04:59 UTC
    assert!(!e.expired(at("2026-11-21T04:59:00Z"), &tz));
    assert!(e.expired(at("2026-11-21T05:00:00Z"), &tz));
    let m = Event::new("x", EventKind::Due, "x").expires("2026-11-20T10:00:00Z");
    assert!(m.expired(at("2026-11-20T10:00:00Z"), &tz));
    assert!(!Event::new("x", EventKind::Due, "x").expired(at("2100-01-01T00:00:00Z"), &tz));
    assert_eq!(When::parse("2026-11-20T23:30:00-05:00").unwrap().day(&tz), NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
    assert_eq!(When::parse("2026-11-20T23:30:00-05:00").unwrap().day(&Utc), NaiveDate::from_ymd_opt(2026, 11, 21).unwrap());
    assert_eq!(When::parse("tomorrow"), None);
}

#[test]
fn dismissals_stick_across_rewrites() {
    let t = tempfile::tempdir().unwrap();
    let root = Root::at(t.path());
    let ev = [Event::new("a", EventKind::Review, "3 receipts to review"), Event::new("b", EventKind::Info, "b")];
    today::publish(&root, "expense", &ev).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap();
    let m = today::read(&root, now);
    today::dismiss(&root, &m.events[0], now).unwrap();
    today::publish(&root, "expense", &ev).unwrap();
    let ids: Vec<String> = today::read(&root, now).events.into_iter().map(|e| e.id).collect();
    assert_eq!(ids, ["b"]);
    // old dismissals are forgotten when a new one is written
    let later = now + chrono::Duration::days(90);
    let m = today::read(&root, later);
    today::dismiss(&root, &m.events[0], later).unwrap();
    let d = today::Dismissals::load(&root);
    assert_eq!(d.0.keys().collect::<Vec<_>>(), ["expense/b"]);
    today::undismiss_all(&root).unwrap();
    assert_eq!(today::read(&root, later).events.len(), 2);
}

#[test]
fn bad_files_and_limits() {
    let t = tempfile::tempdir().unwrap();
    let root = Root::at(t.path());
    let dir = root.today_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.json"), "{").unwrap();
    std::fs::write(dir.join("future.json"), r#"{"schema": 9, "app": "future", "events": []}"#).unwrap();
    std::fs::write(dir.join("Not An App.json"), "{}").unwrap();
    std::fs::write(dir.join(".vigia.json.123.0.tmp"), "{").unwrap();
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    // a file can't post as another app: the file name wins
    std::fs::write(dir.join("rama.json"), r#"{"schema": 1, "app": "vigia", "events": [{"id": "1", "app": "vigia", "kind": "info", "title": "t"}]}"#).unwrap();
    let m = today::read(&root, Utc::now());
    let mut errs: Vec<&str> = m.errors.iter().map(|(f, _)| f.as_str()).collect();
    errs.sort();
    assert_eq!(errs, ["broken.json", "future.json"]);
    assert_eq!(m.events.len(), 1);
    assert_eq!(m.events[0].app, "rama");

    let many: Vec<Event> = (0..300).map(|i| Event::new(format!("e{i}"), EventKind::Info, "x".repeat(500))).collect();
    today::publish(&root, "mosaico", &many).unwrap();
    let m = today::read(&root, Utc::now());
    let mine: Vec<&Event> = m.events.iter().filter(|e| e.app == "mosaico").collect();
    assert_eq!(mine.len(), today::MAX_EVENTS);
    assert_eq!(mine[0].title.chars().count(), today::MAX_TITLE);
    assert!(today::publish(&root, "../evil", &[]).is_err());
    assert!(today::read(&Root::at(t.path().join("missing")), Utc::now()).events.is_empty());
}

#[test]
fn concurrent_publish_never_shows_a_torn_file() {
    let t = tempfile::tempdir().unwrap();
    let root = Root::at(t.path());
    today::publish(&root, "vigia", &[]).unwrap();
    let writer = {
        let root = root.clone();
        std::thread::spawn(move || {
            for i in 0..200 {
                let ev: Vec<Event> = (0..20).map(|j| Event::new(format!("{i}-{j}"), EventKind::Due, "x".repeat(100))).collect();
                today::publish(&root, "vigia", &ev).unwrap();
            }
        })
    };
    for _ in 0..200 {
        let m = today::read(&root, Utc::now());
        assert!(m.errors.is_empty(), "{:?}", m.errors);
    }
    writer.join().unwrap();
}
