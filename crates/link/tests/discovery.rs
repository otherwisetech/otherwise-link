use otherwise_link::apps::{self, Capability, Locations, Os};
use std::fs;
use std::path::Path;

fn plist(id: &str, version: &str) -> String {
    format!("<?xml version=\"1.0\"?><plist><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>")
}

fn mac_app(dir: &Path, name: &str, id: &str, version: &str, bins: &[&str]) {
    let c = dir.join(format!("{name}.app")).join("Contents");
    fs::create_dir_all(c.join("MacOS")).unwrap();
    fs::write(c.join("Info.plist"), plist(id, version)).unwrap();
    for b in bins {
        fs::write(c.join("MacOS").join(b), "").unwrap();
    }
}

#[test]
fn macos_bundles_and_path() {
    let t = tempfile::tempdir().unwrap();
    let (sys, user, bin) = (t.path().join("Applications"), t.path().join("home/Applications"), t.path().join("bin"));
    for d in [&sys, &user, &bin] {
        fs::create_dir_all(d).unwrap();
    }
    mac_app(&sys, "expense", "io.github.otherwisetech.expense", "0.9.1", &["expense", "expense-cli"]);
    mac_app(&user, "Vigía", "io.github.otherwisetech.vigia-app", "0.3.0", &["vigia-app", "vigia"]);
    // same name, someone else's bundle: not ours
    mac_app(&sys, "rama", "com.example.rama", "9.9", &["rama"]);
    // Nodo only as a CLI on PATH
    fs::write(bin.join("nodo"), "").unwrap();
    let loc = Locations { os: Os::Macos, mac_dirs: vec![sys.clone(), user.clone()], roots: vec![], path: vec![bin.clone()] };

    let found = apps::discover(&loc);
    let ids: Vec<&str> = found.iter().map(|(s, _)| s.id.as_str()).collect();
    assert_eq!(ids, ["expense", "vigia", "nodo"]);

    let (spec, exp) = &found[0];
    assert!(spec.has(Capability::Mcp));
    assert_eq!(exp.version.as_deref(), Some("0.9.1"));
    assert_eq!(exp.cli.as_deref(), Some(sys.join("expense.app/Contents/MacOS/expense-cli").as_path()));
    assert_eq!(exp.executable.as_deref(), Some(sys.join("expense.app/Contents/MacOS/expense").as_path()));
    assert_eq!(found[1].1.path.as_deref(), Some(user.join("Vigía.app").as_path()));
    let nodo = &found[2].1;
    assert_eq!((nodo.path.as_ref(), nodo.cli.as_deref()), (None, Some(bin.join("nodo").as_path())));
    assert!(apps::detect_install(&apps::known("nodo").unwrap().spec(), &loc).is_none());
}

#[test]
fn linux_and_windows_folders() {
    let t = tempfile::tempdir().unwrap();
    let (first, second) = (t.path().join("local/opt/otherwise"), t.path().join("opt/otherwise"));
    fs::create_dir_all(first.join("vigia")).unwrap();
    fs::write(first.join("vigia/vigia-app"), "").unwrap();
    fs::write(first.join("vigia/vigia"), "").unwrap();
    fs::write(first.join("vigia/.otherwise-install.json"), r#"{"id":"vigia","version":"v0.4.0","tag":"v0.4.0"}"#).unwrap();
    fs::create_dir_all(second.join("rama")).unwrap();
    fs::write(second.join("rama/rama"), "").unwrap();
    // a marker for another app is ignored
    fs::write(second.join("rama/.otherwise-install.json"), r#"{"id":"other","version":"1"}"#).unwrap();
    let loc = Locations { os: Os::Linux, mac_dirs: vec![], roots: vec![first.clone(), second.clone()], path: vec![] };
    let v = apps::detect(&apps::known("vigia").unwrap().spec(), &loc).unwrap();
    assert_eq!(v.version.as_deref(), Some("0.4.0"));
    assert_eq!(v.cli.as_deref(), Some(first.join("vigia/vigia").as_path()));
    let r = apps::detect(&apps::known("rama").unwrap().spec(), &loc).unwrap();
    assert_eq!((r.version, r.cli), (None, None));

    let win = t.path().join("Programs/Otherwise");
    fs::create_dir_all(win.join("nodo")).unwrap();
    fs::write(win.join("nodo/nodo-app.exe"), "").unwrap();
    fs::write(win.join("nodo/nodo.exe"), "").unwrap();
    // without .exe it isn't a Windows install
    fs::create_dir_all(win.join("rama")).unwrap();
    fs::write(win.join("rama/rama"), "").unwrap();
    let loc = Locations { os: Os::Windows, mac_dirs: vec![], roots: vec![win.clone()], path: vec![] };
    let found: Vec<String> = apps::discover(&loc).into_iter().map(|(s, _)| s.id).collect();
    assert_eq!(found, ["nodo"]);
    assert_eq!(apps::detect(&apps::known("nodo").unwrap().spec(), &loc).unwrap().cli, Some(win.join("nodo/nodo.exe")));
}

#[test]
fn system_locations_have_the_usual_places() {
    let home = Path::new("/home/me");
    let mac = Locations::for_os(Os::Macos, home, vec![]);
    assert_eq!(mac.mac_dirs[1], home.join("Applications"));
    let linux = Locations::for_os(Os::Linux, home, vec![]);
    assert_eq!(linux.roots[0], home.join(".local/opt/otherwise"));
    assert!(!Locations::system().path.is_empty());
}

#[test]
fn apps_describe_themselves_with_otherwise_app_json() {
    let t = tempfile::tempdir().unwrap();
    let sys = t.path().join("Applications");
    fs::create_dir_all(&sys).unwrap();
    let manifest = |id: &str, bundle: &str| format!(r#"{{"schema":1,"id":"{id}","name":"Surco Riego","bundleId":"{bundle}","binary":"surco-riego","cli":["surco-riego"],"mcpArgs":["mcp"],"capabilities":["deepLinks","mcp","somethingNewer"]}}"#);
    let with_manifest = |name: &str, plist_id: &str, json: &str| {
        mac_app(&sys, name, plist_id, "0.2.0", &["surco-riego"]);
        let res = sys.join(format!("{name}.app/Contents/Resources"));
        fs::create_dir_all(&res).unwrap();
        fs::write(res.join(apps::APP_MANIFEST), json).unwrap();
    };
    with_manifest("Surco Riego", "com.otherwisetech.surco.riego", &manifest("surco-riego", "com.otherwisetech.surco.riego"));
    // the bundle id in Info.plist must match the manifest's
    with_manifest("Impostor", "com.example.impostor", &manifest("impostor", "com.otherwisetech.impostor"));
    // not one of ours
    with_manifest("Other", "com.example.other", &manifest("other", "com.example.other"));
    // a known id can't be redefined
    with_manifest("Fake expense", "com.otherwisetech.fake", &manifest("expense", "com.otherwisetech.fake"));
    // newer schema, unreadable
    with_manifest("Newer", "com.otherwisetech.newer", &manifest("newer", "com.otherwisetech.newer").replace("\"schema\":1", "\"schema\":2"));
    with_manifest("Broken", "com.otherwisetech.broken", "{");
    let loc = Locations { os: Os::Macos, mac_dirs: vec![sys.clone()], roots: vec![], path: vec![] };
    let specs = apps::manifest_specs(&loc);
    assert_eq!(specs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["surco-riego"]);
    let s = &specs[0];
    assert_eq!((s.app_name.as_str(), s.binary.as_str()), ("Surco Riego", "surco-riego"));
    assert_eq!(s.capabilities, [Capability::DeepLinks, Capability::Mcp], "unknown capabilities are skipped");
    let found = apps::discover(&loc);
    let (_, inst) = found.iter().find(|(s, _)| s.id == "surco-riego").unwrap();
    assert_eq!(inst.version.as_deref(), Some("0.2.0"));
    assert!(inst.cli.as_ref().unwrap().ends_with("Surco Riego.app/Contents/MacOS/surco-riego"));

    // Linux: next to the binary, in the install folder
    let opt = t.path().join("opt/otherwise");
    fs::create_dir_all(opt.join("surco-abono")).unwrap();
    fs::write(opt.join("surco-abono/surco-abono"), "").unwrap();
    fs::write(opt.join("surco-abono").join(apps::APP_MANIFEST), r#"{"schema":1,"id":"surco-abono","name":"Surco Abono","bundleId":"com.otherwisetech.surco.abono","binary":"surco-abono","binary_extra":"ignored"}"#).unwrap();
    let loc = Locations { os: Os::Linux, mac_dirs: vec![], roots: vec![opt], path: vec![] };
    let found: Vec<String> = apps::discover(&loc).into_iter().map(|(s, _)| s.id).collect();
    assert_eq!(found, ["surco-abono"]);
}

#[test]
fn new_apps_are_known() {
    for (id, bundle) in [("mi-huerta", "com.otherwisetech.mi-huerta"), ("expense-me", "com.otherwisetech.expense-me"), ("surco", "com.otherwisetech.surco")] {
        assert_eq!(apps::known(id).unwrap().bundle_id, bundle);
    }
    let h = apps::known("mi-huerta").unwrap().spec();
    assert!(h.has(Capability::Mcp) && h.has(Capability::Calendar));
    assert_eq!(h.cli, ["mi-huerta-cli"]);
}
