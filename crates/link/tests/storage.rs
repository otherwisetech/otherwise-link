use otherwise_link::cloud::{self, Provider};
use otherwise_link::lock::{self, Holder, Lock, Me, Scope};
use otherwise_link::paths::Home;
use otherwise_link::registry::Registry;
use otherwise_link::relocate::{self, Blocker, Migration, Transfer};
use otherwise_link::settings::HubSettings;
use otherwise_link::storage;
use otherwise_link::{Entity, Kind, Root};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

fn make_db(p: &Path, rows: i64) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let c = Connection::open(p).unwrap();
    c.execute_batch("PRAGMA application_id = 1234; PRAGMA user_version = 3; CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT, b BLOB, r REAL);").unwrap();
    for i in 0..rows {
        c.execute("INSERT INTO t(v, b, r) VALUES (?1, ?2, ?3)", rusqlite::params![format!("row {i}"), vec![i as u8; 3], i as f64 / 3.0]).unwrap();
    }
}

#[test]
fn sqlite_copy_is_verified_and_rollback() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("a.db"), t.path().join("sub").join("b.db"));
    make_db(&src, 50);
    // a WAL source still copies whole, and the copy is in rollback mode
    let c = Connection::open(&src).unwrap();
    let _: String = c.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0)).unwrap();
    c.execute("INSERT INTO t(v) VALUES ('in the wal')", []).unwrap();
    let fp = storage::copy_sqlite(&src, &dest).unwrap();
    assert_eq!(fp.application_id, 1234);
    assert_eq!(fp.user_version, 3);
    assert_eq!(fp.tables, vec![("t".to_string(), 51, fp.tables[0].2)]);
    assert_eq!(storage::verify_sqlite(&dest).unwrap(), storage::fingerprint(&c).unwrap());
    let mode: String = Connection::open(&dest).unwrap().query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(mode, "delete");
    // never overwrites
    assert!(storage::copy_sqlite(&src, &dest).is_err());
    drop(c);
    let left: Vec<_> = std::fs::read_dir(dest.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left.len(), 1, "no temp files left: {left:?}");
}

#[test]
fn sqlite_move_deletes_only_after_verification() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("a.db"), t.path().join("b.db"));
    make_db(&src, 10);
    let before = storage::verify_sqlite(&src).unwrap();
    // destination taken: nothing happens to the source
    std::fs::write(&dest, b"x").unwrap();
    assert!(storage::move_sqlite(&src, &dest).is_err());
    assert!(src.exists());
    std::fs::remove_file(&dest).unwrap();
    // not a database: refused, source kept
    let junk = t.path().join("junk.db");
    std::fs::write(&junk, b"not sqlite at all, just some bytes").unwrap();
    assert!(storage::move_sqlite(&junk, &t.path().join("junk2.db")).is_err());
    assert!(junk.exists());
    storage::move_sqlite(&src, &dest).unwrap();
    assert!(!src.exists());
    assert_eq!(storage::verify_sqlite(&dest).unwrap(), before);
}

#[test]
fn tree_copy_and_move() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("vigia");
    make_db(&src.join("vigia.db"), 5);
    std::fs::create_dir_all(src.join("vigia-files").join("7")).unwrap();
    std::fs::write(src.join("vigia-files").join("7").join("soat.pdf"), b"%PDF").unwrap();
    std::fs::write(src.join("vigia.db-journal"), b"stale").unwrap();
    std::fs::write(src.join("vigia.db.lock.json"), b"{}").unwrap();
    let copied = storage::copy_tree(&src, &t.path().join("copy")).unwrap();
    assert_eq!(copied.len(), 2, "{copied:?}");
    assert!(!t.path().join("copy").join("vigia.db-journal").exists() && !t.path().join("copy").join("vigia.db.lock.json").exists());
    // into an existing file: error and nothing half-copied stays
    let busy = t.path().join("busy");
    std::fs::create_dir_all(busy.join("vigia-files")).unwrap();
    std::fs::write(busy.join("vigia.db"), b"mine").unwrap();
    assert!(storage::copy_tree(&src, &busy).is_err());
    assert_eq!(std::fs::read(busy.join("vigia.db")).unwrap(), b"mine");
    let moved = t.path().join("elsewhere").join("vigia");
    storage::move_tree(&src, &moved).unwrap();
    assert!(!src.exists());
    assert_eq!(std::fs::read(moved.join("vigia-files").join("7").join("soat.pdf")).unwrap(), b"%PDF");
    storage::verify_sqlite(&moved.join("vigia.db")).unwrap();
}

#[test]
fn cloud_detection() {
    let home = PathBuf::from("/Users/ana");
    let d = |p: PathBuf| cloud::detect_in(&p, &home, &[]).map(|c| (c.provider, c.root));
    let cs = home.join("Library").join("CloudStorage");
    assert_eq!(d(cs.join("OneDrive-Personal").join("Otherwise").join("x.db")), Some((Provider::OneDrive, cs.join("OneDrive-Personal"))));
    assert_eq!(d(cs.join("Dropbox").join("a")), Some((Provider::Dropbox, cs.join("Dropbox"))));
    assert_eq!(d(cs.join("GoogleDrive-ana@example.com").join("My Drive").join("a")).unwrap().0, Provider::GoogleDrive);
    assert_eq!(d(cs.join("Box-Box").join("a")).unwrap().0, Provider::Box);
    assert_eq!(d(cs.join("Nextcloud-ana").join("a")).unwrap().0, Provider::Other);
    let md = home.join("Library").join("Mobile Documents").join("com~apple~CloudDocs");
    assert_eq!(d(md.join("Otherwise")), Some((Provider::ICloud, md.clone())));
    assert_eq!(d(home.join("Dropbox (Personal)").join("a")).unwrap().0, Provider::Dropbox);
    assert_eq!(d(home.join("OneDrive - Contoso").join("a")).unwrap().0, Provider::OneDrive);
    assert_eq!(d(home.join("Google Drive").join("a")).unwrap().0, Provider::GoogleDrive);
    assert_eq!(d(home.join("iCloudDrive").join("a")).unwrap().0, Provider::ICloud);
    assert_eq!(d(PathBuf::from("/Volumes/G").join("My Drive").join("a")).unwrap().0, Provider::GoogleDrive);
    assert_eq!(d(home.join(".otherwise")), None);
    assert_eq!(d(home.join("Library").join("Application Support").join("Vigia")), None);
    assert_eq!(d(home.join("Documents").join("Dropbox notes")), None);
    let od = PathBuf::from("/mnt/c/Users/ana/OneDrive - Work");
    assert_eq!(cloud::detect_in(&od.join("a"), &home, std::slice::from_ref(&od)).map(|c| c.provider), Some(Provider::OneDrive));
}

#[test]
fn conflicted_copies() {
    let c = |cand: &str, p: Option<Provider>| cloud::is_conflict_name("vigia.db", cand, p);
    assert!(c("vigia (conflicted copy 2026-11-21).db", None));
    assert!(c("vigia (Ana's conflicted copy 2026-11-21).db", None));
    assert!(c("vigia.sync-conflict-20261121-101010-ABCDEFG.db", None));
    assert!(c("vigia 2.db", None));
    assert!(c("vigia (1).db", None));
    assert!(c("vigia-DESKTOP-1A2B3C.db", Some(Provider::OneDrive)));
    assert!(!c("vigia-DESKTOP-1A2B3C.db", None));
    assert!(!c("vigia.db", None));
    assert!(!c("vigia backup.db", None));
    assert!(!c("vigia 2.txt", None));
    assert!(!c("vigia.db-journal", None));
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("vigia.db");
    for n in ["vigia.db", "vigia 2.db", "vigia (conflicted copy).db", "other.db"] {
        std::fs::write(t.path().join(n), b"").unwrap();
    }
    let found = cloud::conflicted_copies_with(&f, None);
    assert_eq!(found, vec![t.path().join("vigia (conflicted copy).db"), t.path().join("vigia 2.db")]);
    let st = storage::status(&f);
    assert!(st.needs_attention() && st.cloud.is_none() && st.lock == Holder::Free);
}

#[test]
fn lock_heartbeat_and_stale() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("map.nodo");
    let a = Me { host: "laptop".into(), pid: 10 };
    let b = Me { host: "desktop".into(), pid: 20 };
    let alive = |_: u32| true;
    let mut la = Lock::acquire_as(&f, "nodo", Scope::Process, a.clone(), 1000, alive).unwrap();
    assert!(lock::lock_path(&f).exists());
    // another computer sees it as open there
    match Lock::acquire_as(&f, "nodo", Scope::Process, b.clone(), 1100, alive) {
        Err(Holder::Elsewhere(i)) => assert_eq!((i.host.as_str(), i.app.as_str()), ("laptop", "nodo")),
        other => panic!("{other:?}"),
    }
    // the same computer, another live process: here; a dead one: stale
    let a2 = Me { host: "laptop".into(), pid: 11 };
    assert!(matches!(lock::inspect_as(&f, &a2, 1100, alive), Holder::Here(_)));
    assert!(matches!(lock::inspect_as(&f, &a2, 1100, |_| false), Holder::Stale(_)));
    assert!(matches!(lock::inspect_as(&f, &a, 1100, alive), Holder::Mine(_)));
    // heartbeats keep it fresh
    la.heartbeat_at(1500).unwrap();
    assert!(matches!(lock::inspect_as(&f, &b, 1500 + 599, alive), Holder::Elsewhere(_)));
    // no heartbeat for 10 minutes: the other computer may take it
    let stale_at = 1500 + lock::STALE_AFTER.as_secs() as i64;
    assert!(matches!(lock::inspect_as(&f, &b, stale_at, alive), Holder::Stale(_)));
    let lb = Lock::acquire_as(&f, "nodo", Scope::Process, b.clone(), stale_at, alive).unwrap();
    // the first holder notices it lost the lock
    assert!(matches!(la.heartbeat_at(stale_at + 1), Err(Holder::Elsewhere(_))));
    drop(la); // not ours any more: the file stays
    assert_eq!(lock::read(&f).unwrap().host, "desktop");
    lb.release();
    assert!(!lock::lock_path(&f).exists());
}

#[test]
fn computer_lock_is_shared_here() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("entities.db");
    let a = Me { host: "laptop".into(), pid: 10 };
    let a2 = Me { host: "laptop".into(), pid: 11 };
    let b = Me { host: "desktop".into(), pid: 20 };
    let l = Lock::acquire_as(&f, "vigia", Scope::Computer, a, 1000, |_| true).unwrap();
    drop(l);
    assert!(lock::lock_path(&f).exists(), "a computer lock stays (goes stale)");
    assert!(Lock::acquire_as(&f, "nodo", Scope::Computer, a2, 1010, |_| true).is_ok());
    assert!(matches!(Lock::acquire_as(&f, "vigia", Scope::Computer, b.clone(), 1020, |_| true), Err(Holder::Elsewhere(_))));
    assert!(Lock::acquire_as(&f, "vigia", Scope::Computer, b, 1010 + 601, |_| true).is_ok());
}

#[test]
fn registry_refuses_when_another_computer_holds_it() {
    let t = tempfile::tempdir().unwrap();
    let root = Root::at(t.path());
    HubSettings::update(&root, |s| s.entities = true).unwrap();
    Registry::open_shared(&root).unwrap();
    let mut info = lock::read(&root.entities_db()).unwrap();
    assert_eq!(info.scope, Scope::Computer);
    info.host = "someone else's computer".into();
    info.heartbeat = lock::now();
    std::fs::write(lock::lock_path(&root.entities_db()), serde_json::to_string(&info).unwrap()).unwrap();
    assert!(matches!(Registry::open_shared(&root), Err(otherwise_link::registry::Error::InUseElsewhere(_))));
}

fn seed_root(dir: &Path) {
    let root = Root::at(dir);
    HubSettings::update(&root, |s| s.entities = true).unwrap();
    let mut r = Registry::open_shared(&root).unwrap();
    r.upsert("vigia", Entity::new(Kind::Company, "Otherwise Technologies LLC").with_ref("vigia", "owner-1")).unwrap();
    otherwise_link::today::publish(&root, "vigia", &[otherwise_link::today::Event::new("a", otherwise_link::today::EventKind::Due, "SOAT")]).unwrap();
    std::fs::create_dir_all(dir.join("today")).unwrap();
    std::fs::write(dir.join("today").join(".dismissed.json"), b"{}").unwrap();
}

#[test]
fn legacy_migration_is_idempotent_and_loses_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = Home::at(t.path());
    let legacy = home.legacy_root().to_path_buf();
    seed_root(&legacy);
    let before = storage::verify_sqlite(&legacy.join("entities.db")).unwrap();
    let root = home.data_root();
    // something newer already at the root keeps winning; the old one stays put
    std::fs::create_dir_all(root.join("today")).unwrap();
    std::fs::write(root.join("today").join(".dismissed.json"), br#"{"vigia/x":1}"#).unwrap();
    match relocate::migrate_legacy(&legacy, &root).unwrap() {
        Migration::Done(m) => {
            assert_eq!(m.moved, vec!["entities.db", "today/vigia.json", "hub.json"]);
            assert_eq!(m.kept, vec!["today/.dismissed.json"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(storage::verify_sqlite(&root.join("entities.db")).unwrap(), before);
    assert!(!legacy.join("entities.db").exists() && !legacy.join("hub.json").exists());
    assert!(legacy.join("today").join(".dismissed.json").exists(), "conflicting file kept");
    assert!(legacy.join(relocate::MIGRATED_MARKER).exists());
    assert!(HubSettings::load(&Root::at(&root)).entities);
    assert_eq!(Registry::open_shared(&Root::at(&root)).unwrap().search("otherwise", None, 5).unwrap().len(), 1);
    assert_eq!(relocate::migrate_legacy(&legacy, &root).unwrap(), Migration::AlreadyDone);
    // nothing there / same folder
    assert_eq!(relocate::migrate_legacy(&t.path().join("nope"), &root).unwrap(), Migration::Nothing);
    assert_eq!(relocate::migrate_legacy(&root, &root).unwrap(), Migration::Nothing);
}

#[test]
fn interrupted_migration_resumes() {
    let t = tempfile::tempdir().unwrap();
    let home = Home::at(t.path());
    let legacy = home.legacy_root().to_path_buf();
    seed_root(&legacy);
    let root = home.data_root();
    // a previous run copied the registry but died before removing the original
    storage::copy_sqlite(&legacy.join("entities.db"), &root.join("entities.db")).unwrap();
    let Migration::Done(m) = relocate::migrate_legacy(&legacy, &root).unwrap() else { panic!() };
    assert!(m.moved.contains(&"entities.db".to_string()) && m.kept.is_empty(), "{m:?}");
    assert!(!legacy.join("entities.db").exists());
}

#[test]
fn moving_the_data_root() {
    let t = tempfile::tempdir().unwrap();
    let home = Home::at(t.path());
    let old = home.data_root();
    seed_root(&old);
    make_db(&old.join("vigia").join("vigia.db"), 3);
    std::fs::create_dir_all(old.join("vigia").join("vigia-files").join("1")).unwrap();
    std::fs::write(old.join("vigia").join("vigia-files").join("1").join("a.pdf"), b"pdf").unwrap();
    let reg_before = storage::verify_sqlite(&old.join("entities.db")).unwrap();
    let new = t.path().join("OneDrive").join("Otherwise");

    let p = relocate::plan(&old, &new);
    let names: Vec<_> = p.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["hub.json", "today", "vigia", "entities.db"]);
    assert!(!p.blocked());
    assert_eq!(p.items.iter().find(|i| i.name == "vigia").unwrap().files, 2);

    // Vigía open on another computer (fresh lock): blocked, nothing moves
    let info = lock::LockInfo { app: "vigia".into(), host: "other computer".into(), pid: 7, heartbeat: lock::now(), since: lock::now(), ..Default::default() };
    std::fs::write(lock::lock_path(&old.join("vigia").join("vigia.db")), serde_json::to_string(&info).unwrap()).unwrap();
    let p = relocate::plan(&old, &new);
    assert!(matches!(p.items.iter().find(|i| i.name == "vigia").unwrap().blocker, Some(Blocker::InUse(_))));
    assert!(matches!(relocate::relocate(&home, &new, Transfer::Move), Err(relocate::Error::Blocked(_))));
    assert_eq!(home.data_root(), old);
    std::fs::remove_file(lock::lock_path(&old.join("vigia").join("vigia.db"))).unwrap();

    let r = relocate::relocate(&home, &new, Transfer::Move).unwrap();
    assert!(r.left_behind.is_empty(), "{r:?}");
    assert_eq!(home.data_root(), new);
    assert_eq!(storage::verify_sqlite(&new.join("entities.db")).unwrap(), reg_before);
    assert_eq!(std::fs::read(new.join("vigia").join("vigia-files").join("1").join("a.pdf")).unwrap(), b"pdf");
    assert!(HubSettings::load(&Root::at(&new)).entities);
    assert!(!old.join("entities.db").exists() && !old.join("vigia").exists() && !old.join("hub.json").exists());
    assert!(home.pointer_file().exists(), "the pointer stays at its fixed place");

    // back to the default with a copy: both have it
    let r = relocate::relocate(&home, &old, Transfer::Copy).unwrap();
    assert_eq!(r.items.len(), 4);
    assert!(new.join("entities.db").exists() && old.join("entities.db").exists());
    assert_eq!(home.data_root(), old);
    // the destination already has the data: move refused, leave just points there
    assert!(matches!(relocate::relocate(&home, &new, Transfer::Move), Err(relocate::Error::Blocked(p)) if p.items.iter().all(|i| i.blocker == Some(Blocker::Exists))));
    relocate::relocate(&home, &new, Transfer::Leave).unwrap();
    assert_eq!(home.data_root(), new);
    assert!(matches!(relocate::relocate(&home, &new, Transfer::Move), Err(relocate::Error::Same)));
    assert!(matches!(relocate::relocate(&home, &new.join("vigia").join("x"), Transfer::Move), Err(relocate::Error::Inside(n)) if n == "vigia"));
}
