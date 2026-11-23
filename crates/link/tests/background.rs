use otherwise_link::Root;
use otherwise_link::apps::{self, Locations, Os};
use otherwise_link::background::{self, Job, Label, Manifest, NotRunnable, Notification};
use std::fs;
use std::path::{Path, PathBuf};

const EXPENSE: &str = "io.github.otherwisetech.expense";

fn manifest(app: &str) -> Manifest {
    let mut mail = Job::every("mail", Label::en("background.mail", "Check mail").with("es", "Revisar correo"), "15m", &["background", "run", "mail"]);
    mail.heavy = true;
    mail.network = true;
    let mut bills = Job::every("bills", Label::en("background.bills", "Bills due"), "1h", &["background", "run", "bills"]);
    bills.exe = Some("expense-cli".into());
    Manifest { schema: 1, app: app.into(), jobs: vec![mail, bills] }
}

#[cfg(unix)]
fn make_exec(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
}
#[cfg(not(unix))]
fn make_exec(_: &Path) {}

/// `<dir>/expense.app` with its binaries and (optionally) its manifest.
fn install_expense(dir: &Path, bundle_id: &str, with_manifest: bool) -> PathBuf {
    let app = dir.join("expense.app");
    let c = app.join("Contents");
    fs::create_dir_all(c.join("MacOS")).unwrap();
    fs::create_dir_all(c.join("Resources")).unwrap();
    fs::write(c.join("Info.plist"), format!("<plist><dict><key>CFBundleIdentifier</key><string>{bundle_id}</string><key>CFBundleShortVersionString</key><string>0.6.1</string></dict></plist>")).unwrap();
    for b in ["expense", "expense-cli"] {
        fs::write(c.join("MacOS").join(b), "#!/bin/sh\n").unwrap();
        make_exec(&c.join("MacOS").join(b));
    }
    if with_manifest {
        fs::write(c.join("Resources").join(background::MANIFEST), manifest("expense").to_json()).unwrap();
    }
    app
}

fn mac(t: &Path) -> Locations {
    let sys = t.join("Applications");
    fs::create_dir_all(&sys).unwrap();
    Locations { os: Os::Macos, mac_dirs: vec![sys], roots: vec![], path: vec![] }
}

#[test]
fn scan_finds_only_installed_apps_that_declare_jobs() {
    let t = tempfile::tempdir().unwrap();
    let loc = mac(t.path());
    // installed without a manifest: no jobs
    install_expense(&loc.mac_dirs[0], EXPENSE, false);
    assert!(background::scan(&loc).is_empty());
    fs::write(loc.mac_dirs[0].join("expense.app/Contents/Resources").join(background::MANIFEST), manifest("expense").to_json()).unwrap();
    let found = background::scan(&loc);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].spec.id, "expense");
    assert_eq!(found[0].manifest.jobs.len(), 2);
    let p = background::plan(&found[0], "mail", &loc).unwrap();
    assert_eq!(p.program, loc.mac_dirs[0].join("expense.app/Contents/MacOS/expense"));
    assert_eq!(p.args, ["background", "run", "mail"]);
    let p = background::plan(&found[0], "bills", &loc).unwrap();
    assert!(p.program.ends_with("Contents/MacOS/expense-cli"));
    assert_eq!(background::plan(&found[0], "nope", &loc), Err(NotRunnable::NoSuchJob));
}

#[test]
fn a_manifest_without_its_app_is_ignored_and_left_alone() {
    let t = tempfile::tempdir().unwrap();
    let loc = mac(t.path());
    let root = Root::at(t.path().join("data"));
    // a stale declaration in the data root (an old version, or the app was deleted)
    let stale = root.app_dir("expense").join(background::MANIFEST);
    fs::create_dir_all(stale.parent().unwrap()).unwrap();
    fs::write(&stale, manifest("expense").to_json()).unwrap();
    assert!(background::scan(&loc).is_empty());
    assert!(stale.exists(), "the user's data folder is never cleaned up");
    // someone else's bundle with our name and a manifest: not ours
    install_expense(&loc.mac_dirs[0], "com.example.expense", true);
    assert!(background::scan(&loc).is_empty());
    // a manifest for another app id inside our bundle
    let dir = t.path().join("other");
    let app = install_expense(&dir, EXPENSE, false);
    fs::write(app.join("Contents/Resources").join(background::MANIFEST), manifest("vigia").to_json()).unwrap();
    let loc2 = Locations { os: Os::Macos, mac_dirs: vec![dir], roots: vec![], path: vec![] };
    assert!(background::scan(&loc2).is_empty());
}

#[test]
fn missing_or_bad_executables_are_not_run() {
    let t = tempfile::tempdir().unwrap();
    let loc = mac(t.path());
    let app = install_expense(&loc.mac_dirs[0], EXPENSE, true);
    let reg = background::scan(&loc).remove(0);
    fs::remove_file(app.join("Contents/MacOS/expense-cli")).unwrap();
    assert!(matches!(background::plan(&reg, "bills", &loc), Err(NotRunnable::BadExecutable(_))));
    #[cfg(unix)]
    {
        // not executable
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(app.join("Contents/MacOS/expense"), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(background::plan(&reg, "mail", &loc), Err(NotRunnable::BadExecutable(_))));
        // a symlink out of the bundle
        let outside = t.path().join("sh");
        fs::write(&outside, "").unwrap();
        make_exec(&outside);
        std::os::unix::fs::symlink(&outside, app.join("Contents/MacOS/expense-cli")).unwrap();
        assert!(matches!(background::plan(&reg, "bills", &loc), Err(NotRunnable::BadExecutable(_))));
    }
}

#[test]
fn removed_while_running_then_reinstalled() {
    let t = tempfile::tempdir().unwrap();
    let loc = mac(t.path());
    let app = install_expense(&loc.mac_dirs[0], EXPENSE, true);
    let reg = background::scan(&loc).remove(0);
    assert!(background::plan(&reg, "mail", &loc).is_ok());
    // dragged to the Trash while the helper runs
    let trash = t.path().join("Trash");
    fs::create_dir_all(&trash).unwrap();
    fs::rename(&app, trash.join("expense.app")).unwrap();
    assert_eq!(background::plan(&reg, "mail", &loc), Err(NotRunnable::NotInstalled));
    assert!(background::scan(&loc).is_empty());
    // moved to ~/Applications: a different install, the old registration doesn't follow it
    let user = t.path().join("home/Applications");
    fs::create_dir_all(&user).unwrap();
    fs::rename(trash.join("expense.app"), user.join("expense.app")).unwrap();
    let loc_both = Locations { os: Os::Macos, mac_dirs: vec![loc.mac_dirs[0].clone(), user.clone()], roots: vec![], path: vec![] };
    assert_eq!(background::plan(&reg, "mail", &loc_both), Err(NotRunnable::NotInstalled));
    let moved = background::scan(&loc_both).remove(0);
    assert!(background::plan(&moved, "mail", &loc_both).is_ok());
    // reinstalled in /Applications: found again by the next scan
    fs::remove_dir_all(user.join("expense.app")).unwrap();
    install_expense(&loc.mac_dirs[0], EXPENSE, true);
    let again = background::scan(&loc);
    assert_eq!(again.len(), 1);
    assert!(background::plan(&again[0], "mail", &loc).is_ok());
    // an update that drops a job
    let mut m = manifest("expense");
    m.jobs.retain(|j| j.id == "mail");
    fs::write(loc.mac_dirs[0].join("expense.app/Contents/Resources").join(background::MANIFEST), m.to_json()).unwrap();
    assert_eq!(background::plan(&again[0], "bills", &loc), Err(NotRunnable::NoSuchJob));
}

#[test]
fn linux_install_folders() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("opt/otherwise");
    let dir = root.join("vigia");
    fs::create_dir_all(&dir).unwrap();
    for b in ["vigia-app", "vigia"] {
        fs::write(dir.join(b), "").unwrap();
        make_exec(&dir.join(b));
    }
    let mut m = manifest("vigia");
    m.jobs[0].exe = Some("vigia".into());
    fs::write(dir.join(background::MANIFEST), m.to_json()).unwrap();
    let loc = Locations { os: Os::Linux, mac_dirs: vec![], roots: vec![root], path: vec![] };
    let reg = background::scan(&loc);
    assert_eq!(reg.len(), 1);
    assert_eq!(background::plan(&reg[0], "mail", &loc).unwrap().program, dir.join("vigia"));
    // a CLI on PATH alone has no install folder: no background jobs
    let bin = t.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("nodo"), "").unwrap();
    let loc = Locations { os: Os::Linux, mac_dirs: vec![], roots: vec![], path: vec![bin] };
    assert!(apps::detect(&apps::known("nodo").unwrap().spec(), &loc).is_some());
    assert!(background::scan(&loc).is_empty());
}

#[test]
fn outbox_roundtrip() {
    let t = tempfile::tempdir().unwrap();
    let root = Root::at(t.path());
    let now = chrono::Utc::now();
    assert!(!background::pending(&root, "expense"));
    background::notify(&root, "expense", &Notification::new("bill-42", "Internet bill due tomorrow").body("$120.000")).unwrap();
    // same id again replaces the pending one
    background::notify(&root, "expense", &Notification::new("bill-42", "Internet bill due today")).unwrap();
    let mut old = Notification::new("old", "stale");
    old.created_at = (now - chrono::Duration::days(3)).to_rfc3339();
    background::notify(&root, "expense", &old).unwrap();
    fs::write(background::outbox_dir(&root, "expense").join("junk.json"), "{").unwrap();
    assert!(background::pending(&root, "expense"));
    let got = background::drain(&root, "expense", now);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].app.as_str(), got[0].title.as_str(), got[0].body.as_deref()), ("expense", "Internet bill due today", None));
    assert_eq!(got[0].key(), "expense/bill-42");
    assert!(!background::pending(&root, "expense"), "drained, junk and stale ones removed");
    assert!(background::drain(&root, "vigia", now).is_empty());
}

#[test]
fn windows_install_folders_run_exe_files_only() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Programs/Otherwise");
    let dir = root.join("vigia");
    fs::create_dir_all(&dir).unwrap();
    for b in ["vigia-app.exe", "vigia.exe"] {
        fs::write(dir.join(b), "").unwrap();
    }
    let mut m = manifest("vigia");
    m.jobs[0].exe = Some("vigia".into());
    m.jobs[1].exe = Some("vigia.exe".into());
    fs::write(dir.join(background::MANIFEST), m.to_json()).unwrap();
    let loc = Locations { os: Os::Windows, mac_dirs: vec![], roots: vec![root], path: vec![] };
    let reg = background::scan(&loc).remove(0);
    assert_eq!(background::plan(&reg, "mail", &loc).unwrap().program, dir.join("vigia.exe"));
    assert_eq!(background::plan(&reg, "bills", &loc).unwrap().program, dir.join("vigia.exe"));
    fs::remove_file(dir.join("vigia.exe")).unwrap();
    assert!(matches!(background::plan(&reg, "mail", &loc), Err(NotRunnable::BadExecutable(_))));
}

#[test]
fn one_computer_runs_an_apps_jobs_per_data_root() {
    use otherwise_link::background::{Lease, take_lease_as};
    use otherwise_link::lock::{Me, STALE_AFTER};
    let t = tempfile::tempdir().unwrap();
    let root = Root::at(t.path());
    let mac_a = || Me { host: "Mac A".into(), pid: 100 };
    let mac_b = || Me { host: "Mac B".into(), pid: 200 };
    let t0 = 1_800_000_000;
    assert!(take_lease_as(&root, "expense", mac_a(), t0).held());
    // the other computer on the same synced root waits, and knows who runs them
    match take_lease_as(&root, "expense", mac_b(), t0 + 30) {
        Lease::Elsewhere(host) => assert_eq!(host, "Mac A"),
        other => panic!("{other:?}"),
    }
    // other apps are independent
    assert!(take_lease_as(&root, "vigia", mac_b(), t0 + 30).held());
    // a restarted helper on the same computer keeps it (another pid)
    assert!(take_lease_as(&root, "expense", Me { host: "Mac A".into(), pid: 101 }, t0 + 90).held());
    // Mac A refreshes it…
    assert!(take_lease_as(&root, "expense", mac_a(), t0 + 400).held());
    assert!(!take_lease_as(&root, "expense", mac_b(), t0 + 400 + 300).held());
    // …then goes away: the lease lapses and Mac B takes over
    let later = t0 + 400 + STALE_AFTER.as_secs() as i64 + 1;
    assert!(take_lease_as(&root, "expense", mac_b(), later).held());
    assert!(!take_lease_as(&root, "expense", mac_a(), later + 10).held());
    // the lease file is in the app's folder, next to its outbox
    assert!(background::lease_target(&root, "expense").with_file_name("background.lock.json").exists());
}

#[test]
fn another_computers_lease_doesnt_block_moving_the_root_and_device_state_stays() {
    use otherwise_link::lock::Me;
    use otherwise_link::paths::Home;
    let t = tempfile::tempdir().unwrap();
    let home = Home::at(t.path());
    let root = Root::at(home.default_root());
    fs::create_dir_all(root.app_dir("expense")).unwrap();
    fs::write(root.app_dir("expense").join("data.txt"), "x").unwrap();
    background::notify(&root, "expense", &Notification::new("n", "Queued")).unwrap();
    assert!(background::take_lease_as(&root, "expense", Me { host: "Other Mac".into(), pid: 1 }, otherwise_link::lock::now()).held());
    // what an older helper left in the default root is the device's, not data
    fs::create_dir_all(root.dir().join("helper")).unwrap();
    fs::write(root.dir().join("helper").join("state.json"), "{}").unwrap();
    // a device folder inside the root (odd choice of root) isn't data either
    let to = t.path().join("OneDrive").join("Otherwise");
    let p = otherwise_link::relocate::plan_excluding(root.dir(), &to, &[root.dir().join("device-here")]);
    fs::create_dir_all(root.dir().join("device-here")).unwrap();
    let p2 = otherwise_link::relocate::plan_excluding(root.dir(), &to, &[root.dir().join("device-here")]);
    for plan in [&p, &p2] {
        let names: Vec<&str> = plan.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["expense"], "{names:?}");
        assert!(!plan.blocked(), "a lease is not an open file: {:?}", plan.items);
    }
    let r = otherwise_link::relocate::relocate(&home, &to, otherwise_link::relocate::Transfer::Move).unwrap();
    let _ = r;
    assert!(to.join("expense").join("data.txt").exists());
    assert!(to.join("expense").join("outbox").join("n.json").exists(), "the outbox travels with the data");
    assert!(!to.join("helper").exists());
    assert!(!to.join("expense").join("background.lock.json").exists(), "locks never move");
    assert!(!home.device.starts_with(root.dir()));
}

#[test]
fn self_described_apps_have_jobs_too() {
    let t = tempfile::tempdir().unwrap();
    let loc = mac(t.path());
    let app = loc.mac_dirs[0].join("Surco Riego.app");
    let c = app.join("Contents");
    fs::create_dir_all(c.join("MacOS")).unwrap();
    fs::create_dir_all(c.join("Resources")).unwrap();
    fs::write(c.join("Info.plist"), "<plist><dict><key>CFBundleIdentifier</key><string>com.otherwisetech.surco.riego</string></dict></plist>").unwrap();
    fs::write(c.join("MacOS/surco-riego"), "#!/bin/sh\n").unwrap();
    make_exec(&c.join("MacOS/surco-riego"));
    fs::write(c.join("Resources").join(apps::APP_MANIFEST), r#"{"schema":1,"id":"surco-riego","name":"Surco Riego","bundleId":"com.otherwisetech.surco.riego","binary":"surco-riego"}"#).unwrap();
    let jobs = Manifest { schema: 1, app: "surco-riego".into(), jobs: vec![Job::every("forecast", Label::en("k", "Forecast"), "6h", &["background", "run", "forecast"])] };
    fs::write(c.join("Resources").join(background::MANIFEST), jobs.to_json()).unwrap();
    let found = background::scan(&loc);
    assert_eq!(found.iter().map(|r| r.spec.id.as_str()).collect::<Vec<_>>(), ["surco-riego"]);
    let plan = background::plan(&found[0], "forecast", &loc).unwrap();
    assert!(plan.program.ends_with("Surco Riego.app/Contents/MacOS/surco-riego"));
}
