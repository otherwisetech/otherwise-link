use otherwise_link::entity::{Company, Device, Entity, Kind, Person};
use otherwise_link::registry::{Error, Matched, Op, Registry};
use otherwise_link::settings::HubSettings;
use otherwise_link::tax::TaxId;
use otherwise_link::Root;
use std::path::Path;
use uuid::Uuid;

fn company(name: &str, nit: &str) -> Entity {
    let mut e = Entity::new(Kind::Company, name);
    e.company = Some(Company { tax_ids: vec![TaxId::new("CO", "NIT", nit)], country: Some("CO".into()), ..Default::default() });
    e
}

fn open(dir: &Path) -> Registry {
    Registry::open(&dir.join("entities.db")).unwrap()
}

#[test]
fn opt_in_is_required() {
    let t = tempfile::tempdir().unwrap();
    let root = Root::at(t.path());
    assert!(matches!(Registry::open_shared(&root), Err(Error::Disabled)));
    assert!(!root.entities_db().exists(), "nothing is created before the user opts in");
    HubSettings::update(&root, |s| s.entities = true).unwrap();
    let r = Registry::open_shared(&root).unwrap();
    assert_eq!(r.path(), root.entities_db());
}

#[test]
fn crud_and_matching() {
    let t = tempfile::tempdir().unwrap();
    let mut r = open(t.path());
    let a = r.upsert("expense", company("ACME S.A.S.", "800197268-4").with_ref("expense", "v1")).unwrap();
    assert_eq!(a.matched, Matched::Created);
    assert!(!a.id.is_nil());
    assert!(a.entity.created_at > 0);

    // same NIT spelled differently, from another app: the same company
    let b = r.upsert("vigia", company("Acme", "8001972684").with_ref("vigia", "e7")).unwrap();
    assert_eq!((b.id, b.matched), (a.id, Matched::Key));
    let e = r.get(a.id).unwrap().unwrap();
    assert_eq!(e.name, "Acme", "the newer write wins on scalars");
    assert_eq!(e.refs.len(), 2);

    // by own ref, by another app's ref, by id
    assert_eq!(r.upsert("expense", Entity::new(Kind::Company, "ACME").with_ref("expense", "v1")).unwrap().matched, Matched::OwnRef);
    assert_eq!(r.upsert("nodo", Entity::new(Kind::Company, "ACME").with_ref("vigia", "e7")).unwrap().matched, Matched::OtherRef);
    let mut by_id = Entity::new(Kind::Company, "ACME");
    by_id.id = a.id;
    assert_eq!(r.upsert("nodo", by_id).unwrap().matched, Matched::Id);
    assert_eq!(r.by_ref("vigia", "e7").unwrap().unwrap().id, a.id);
    assert_eq!(r.by_key("tax:CO:NIT:8001972684").unwrap().unwrap().id, a.id);

    // a ref that matches a card of another kind doesn't hijack it
    let p = r.upsert("vigia", Entity::new(Kind::Person, "Ana").with_ref("vigia", "e7")).unwrap();
    assert_eq!(p.matched, Matched::Created);
    let mut wrong = Entity::new(Kind::Person, "x");
    wrong.id = a.id;
    assert!(matches!(r.upsert("vigia", wrong), Err(Error::KindMismatch { .. })));

    // replace removes what upsert would keep
    let mut edited = r.get(a.id).unwrap().unwrap();
    edited.refs.remove("vigia");
    edited.company.as_mut().unwrap().tax_ids.clear();
    r.replace("otherwise-apps", edited).unwrap();
    assert!(r.by_key("tax:CO:NIT:8001972684").unwrap().is_none());
    // refs are unique per app: the person never got "e7" (first come), and the company let it go
    assert!(r.by_ref("vigia", "e7").unwrap().is_none());

    // link / unlink
    let linked = r.link("nodo", a.id, "dev-1").unwrap();
    assert_eq!(linked.refs["nodo"], "dev-1");
    assert_eq!(r.referencing_apps(a.id).unwrap(), [("expense".to_string(), "v1".to_string()), ("nodo".into(), "dev-1".into())]);
    r.unlink("nodo", a.id).unwrap();
    assert!(r.by_ref("nodo", "dev-1").unwrap().is_none());

    // delete
    assert!(r.delete("expense", p.id).unwrap());
    assert!(r.get(p.id).unwrap().is_none());
    assert!(!r.delete("expense", p.id).unwrap());
    assert!(matches!(r.link("x", p.id, "1"), Err(Error::NotFound(_))));
    assert!(matches!(r.upsert("Bad App", company("x", "1")), Err(Error::BadApp(_))));
    assert_eq!(r.counts().unwrap().get(&Kind::Company), Some(&1));
    assert_eq!(r.counts().unwrap().get(&Kind::Person), None);
}

#[test]
fn search_lists_and_duplicates() {
    let t = tempfile::tempdir().unwrap();
    let mut r = open(t.path());
    let jose = r.upsert("vigia", Entity::new(Kind::Person, "José Pérez").with_alias("Pepe")).unwrap().id;
    let jose2 = r.upsert("expense", Entity::new(Kind::Person, "Jose Perez")).unwrap().id;
    r.upsert("expense", Entity::new(Kind::Company, "José Pérez")).unwrap();
    let mut nas = Entity::new(Kind::Device, "NAS");
    nas.device = Some(Device { macs: vec!["aa:bb:cc:dd:ee:ff".into()], hostnames: vec!["storage.lan".into()], ..Default::default() });
    r.upsert("nodo", nas).unwrap();
    // same name, different NIT: not a duplicate
    r.upsert("expense", company("Tienda", "800197268")).unwrap();
    r.upsert("expense", company("Tienda", "900373913")).unwrap();

    assert_eq!(r.search("jose", None, 10).unwrap().len(), 3);
    assert_eq!(r.search("PEREZ jose", Some(Kind::Person), 10).unwrap().len(), 2);
    assert_eq!(r.search("pepe", None, 10).unwrap()[0].id, jose);
    assert_eq!(r.search("aa:bb:cc", None, 10).unwrap()[0].name, "NAS");
    assert_eq!(r.search("storage", None, 10).unwrap()[0].name, "NAS");
    assert_eq!(r.search("", Some(Kind::Device), 10).unwrap().len(), 1);
    assert!(r.search("nobody", None, 10).unwrap().is_empty());
    assert_eq!(r.list(None, 2, 0).unwrap().len(), 2);
    assert_eq!(r.list(None, 100, 0).unwrap().len(), 6);

    let d = r.duplicates(10).unwrap();
    assert_eq!(d.len(), 1, "{:?}", d.iter().map(|(a, b)| (&a.name, &b.name)).collect::<Vec<_>>());
    let ids = [d[0].0.id, d[0].1.id];
    assert!(ids.contains(&jose) && ids.contains(&jose2));
}

#[test]
fn merge_keeps_old_ids_resolving() {
    let t = tempfile::tempdir().unwrap();
    let mut r = open(t.path());
    let mut a = Entity::new(Kind::Person, "Ana Pérez").with_ref("vigia", "p1");
    a.person = Some(Person { emails: vec!["ana@x.co".into()], ..Default::default() });
    let a = r.upsert("vigia", a).unwrap().id;
    let mut b = Entity::new(Kind::Person, "Ana P").with_ref("expense", "c1");
    b.person = Some(Person { emails: vec!["ana@y.co".into()], country: Some("CO".into()), ..Default::default() });
    let b = r.upsert("expense", b).unwrap().id;
    let c = r.upsert("genealogy", Entity::new(Kind::Person, "Ana")).unwrap().id;

    let kept = r.merge("otherwise-apps", a, b).unwrap();
    assert_eq!(kept.id, a);
    assert_eq!(kept.refs.len(), 2);
    assert_eq!(kept.aliases, ["Ana P"]);
    assert_eq!(kept.person.as_ref().unwrap().country.as_deref(), Some("CO"));
    assert_eq!(r.get(b).unwrap().unwrap().id, a, "the merged-away id resolves to the survivor");
    assert_eq!(r.by_ref("expense", "c1").unwrap().unwrap().id, a);
    assert_eq!(r.by_key("email:ana@y.co").unwrap().unwrap().id, a);
    // chains: merge the survivor into a third card; the first id still resolves
    r.merge("otherwise-apps", c, a).unwrap();
    assert_eq!(r.resolve(b).unwrap(), Some(c));
    assert_eq!(r.get(a).unwrap().unwrap().id, c);
    assert_eq!(r.merge("otherwise-apps", c, b).unwrap().id, c, "merging into itself is a no-op");
    assert!(matches!(r.merge("x", c, Uuid::new_v4()), Err(Error::NotFound(_))));
    let co = r.upsert("x", company("Co", "1")).unwrap().id;
    assert!(matches!(r.merge("x", c, co), Err(Error::KindMismatch { .. })));
    assert_eq!(r.list(Some(Kind::Person), 10, 0).unwrap().len(), 1);
}

#[test]
fn sync_cursors() {
    let t = tempfile::tempdir().unwrap();
    let mut r = open(t.path());
    assert_eq!(r.cursor("vigia").unwrap(), 0);
    let x = r.upsert("expense", company("X", "1")).unwrap().id;
    let y = r.upsert("expense", company("Y", "2")).unwrap().id;
    r.upsert("expense", company("X2", "1")).unwrap(); // same NIT: updates x
    let own = r.upsert("vigia", Entity::new(Kind::Person, "Mine")).unwrap().id;

    let p = r.pull("vigia", 100).unwrap();
    assert!(!p.more);
    assert_eq!(p.cursor, 4);
    let got: Vec<(Uuid, &str)> = p.items.iter().map(|s| (s.change.id, s.entity.as_ref().unwrap().name.as_str())).collect();
    assert_eq!(got, [(y, "Y"), (x, "X2")], "collapsed per entity, own changes skipped, ordered by last change");
    r.ack("vigia", p.cursor).unwrap();
    assert!(r.pull("vigia", 100).unwrap().items.is_empty());
    // acks never move a cursor back
    r.ack("vigia", 1).unwrap();
    assert_eq!(r.cursor("vigia").unwrap(), 4);

    // another app edits vigia's card, then vigia edits it again: vigia still hears about it
    r.link("nodo", own, "n1").unwrap();
    r.upsert("vigia", Entity::new(Kind::Person, "Mine").with_ref("vigia", "v1").with_ref("nodo", "n1")).unwrap();
    let p = r.pull("vigia", 100).unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].entity.as_ref().unwrap().refs["nodo"], "n1");
    r.ack("vigia", p.cursor).unwrap();

    // deletes and merges come through without a card
    r.delete("expense", y).unwrap();
    let z = r.upsert("expense", company("Z", "3")).unwrap().id;
    r.merge("expense", x, z).unwrap();
    let p = r.pull("vigia", 100).unwrap();
    let ops: Vec<(Op, Uuid, Option<Uuid>, bool)> = p.items.iter().map(|s| (s.change.op, s.change.id, s.change.into, s.entity.is_some())).collect();
    assert_eq!(ops, [(Op::Delete, y, None, false), (Op::Merge, z, Some(x), false), (Op::Upsert, x, None, true)]);

    // paging
    let first = r.pull("genealogy", 2).unwrap();
    assert!(first.more);
    assert_eq!(first.cursor, 2);
    r.ack("genealogy", first.cursor).unwrap();
    let rest = r.pull("genealogy", 1000).unwrap();
    assert!(!rest.more);
    assert_eq!(rest.cursor, r.last_seq().unwrap());
    assert_eq!(r.changes_since(0, 1000).unwrap().len() as i64, r.last_seq().unwrap());
}

#[test]
fn concurrent_writers_in_separate_connections() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("entities.db");
    drop(Registry::open(&path).unwrap());
    let apps = ["expense", "vigia", "nodo", "genealogy"];
    let handles: Vec<_> = apps
        .iter()
        .enumerate()
        .map(|(n, app)| {
            let path = path.clone();
            let app = app.to_string();
            std::thread::spawn(move || {
                let mut r = Registry::open(&path).unwrap();
                for i in 0..40 {
                    // each app adds its own companies and races the others on a shared one
                    r.upsert(&app, company(&format!("{app} {i}"), &format!("{}{:03}", n + 1, i))).unwrap();
                    r.upsert(&app, company("Shared", "800197268-4").with_ref(&app, "shared")).unwrap();
                    let _ = r.search("shared", None, 5).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let r = Registry::open(&path).unwrap();
    assert_eq!(r.counts().unwrap()[&Kind::Company], apps.len() * 40 + 1, "the shared NIT stayed one company");
    let shared = r.by_key("tax:CO:NIT:8001972684").unwrap().unwrap();
    assert_eq!(shared.refs.len(), apps.len());
    let log = r.changes_since(0, 100_000).unwrap();
    assert_eq!(log.len(), apps.len() * 80);
    assert!(log.iter().enumerate().all(|(i, c)| c.seq == i as i64 + 1), "seq has no gaps");
}

#[test]
fn foreign_and_newer_files_are_refused() {
    let t = tempfile::tempdir().unwrap();
    let other = t.path().join("other.db");
    rusqlite::Connection::open(&other).unwrap().execute_batch("CREATE TABLE x(a);").unwrap();
    assert!(matches!(Registry::open(&other), Err(Error::NotOurs)));
    let newer = t.path().join("newer.db");
    drop(Registry::open(&newer).unwrap());
    rusqlite::Connection::open(&newer).unwrap().execute_batch("PRAGMA user_version = 99;").unwrap();
    assert!(matches!(Registry::open(&newer), Err(Error::NewerDatabase(99))));
    // reopening ours is fine and keeps the data
    let p = t.path().join("e.db");
    let id = Registry::open(&p).unwrap().upsert("x", company("A", "1")).unwrap().id;
    assert!(Registry::open(&p).unwrap().get(id).unwrap().is_some());
    let mode: String = rusqlite::Connection::open(&p).unwrap().query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(mode, "delete");
}

/// Runs as a child process of `concurrent_processes` (does nothing in a normal test run).
#[test]
fn child_writer() {
    let (Ok(path), Ok(app)) = (std::env::var("OWL_CHILD_DB"), std::env::var("OWL_CHILD_APP")) else { return };
    let mut r = Registry::open(Path::new(&path)).unwrap();
    for i in 0..30 {
        r.upsert(&app, company(&format!("{app} {i}"), &format!("{app}{i}"))).unwrap();
        r.upsert(&app, company("Shared", "900373913").with_ref(&app, "s")).unwrap();
    }
    let p = r.pull(&app, 10_000).unwrap();
    r.ack(&app, p.cursor).unwrap();
}

#[test]
fn concurrent_processes() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("entities.db");
    let exe = std::env::current_exe().unwrap();
    let apps = ["expense", "vigia", "nodo"];
    let children: Vec<_> = apps
        .iter()
        .map(|app| std::process::Command::new(&exe).args(["child_writer", "--exact", "--test-threads=1", "--quiet"]).env("OWL_CHILD_DB", &path).env("OWL_CHILD_APP", app).stdout(std::process::Stdio::null()).spawn().unwrap())
        .collect();
    for mut c in children {
        assert!(c.wait().unwrap().success());
    }
    let r = Registry::open(&path).unwrap();
    assert_eq!(r.counts().unwrap()[&Kind::Company], apps.len() * 30 + 1);
    assert_eq!(r.by_key("tax:CO:NIT:900373913").unwrap().unwrap().refs.len(), apps.len());
    assert_eq!(r.last_seq().unwrap(), (apps.len() * 60) as i64);
    for app in apps {
        assert!(r.cursor(app).unwrap() > 0);
    }
}
