use otherwise_link::Root;
use otherwise_link::worktrees::{self, Blocker, Cache, CacheKind, Error, Repo, Report, Worktree};
use std::path::{Path, PathBuf};

fn p(s: &str) -> PathBuf {
    if cfg!(windows) { PathBuf::from(format!("C:{}", s.replace('/', "\\"))) } else { PathBuf::from(s) }
}

fn repo(root: &str, wts: Vec<Worktree>) -> Repo {
    Repo { root: p(root), name: "shop".into(), updated_at: String::new(), default_branch: Some("origin/main".into()), reclaimable: 7, link: Some("otherwise://rama/reclaim?path=x".into()), worktrees: wts, shared: vec![] }
}

fn wt(path: &str, caches: Vec<Cache>, blockers: Vec<Blocker>) -> Worktree {
    Worktree { path: p(path), branch: Some("fix".into()), main: false, bytes: Some(100), touched: Some(1), removable: blockers.is_empty(), blockers, caches }
}

fn cache(path: &str, kind: CacheKind, bytes: u64) -> Cache {
    Cache { path: p(path), kind, bytes, offered: false }
}

#[test]
fn host_keys_are_file_safe() {
    assert_eq!(worktrees::host_key("Juan's MacBook Pro"), "juan-s-macbook-pro");
    assert_eq!(worktrees::host_key("  --  "), "unknown");
    assert_eq!(worktrees::host_key("Café/../x"), "caf-x");
    assert!(worktrees::host_key(&"a".repeat(200)).len() <= 64);
    let root = Root::at(p("/r"));
    assert_eq!(worktrees::file(&root, "rama", "Studio"), p("/r").join("rama").join("worktrees").join("studio.json"));
}

#[test]
fn round_trip_and_per_computer_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = Root::at(dir.path());
    let exists = |_: &Path| true;
    let a = repo("/c/shop", vec![wt("/c/shop-fix", vec![cache("/c/shop-fix/target", CacheKind::Target, 90)], vec![Blocker::Unpushed])]);
    let out = worktrees::upsert_for(&root, "rama", "studio", a.clone(), "2026-11-28T10:00:00Z", &exists).unwrap();
    assert_eq!(out.repos.len(), 1);
    let back = worktrees::read_for(&root, "rama", "studio").unwrap().unwrap();
    assert_eq!(back.repos[0].worktrees[0].caches[0].kind, CacheKind::Target);
    assert_eq!(back.repos[0].worktrees[0].blockers, [Blocker::Unpushed]);
    assert_eq!(back.worktree(&p("/c/shop-fix")).map(|(r, _)| r.root.clone()), Some(p("/c/shop")));
    assert_eq!(back.paths(), [p("/c/shop-fix")]);
    // another computer sharing the root has its own file
    assert!(worktrees::read_for(&root, "rama", "laptop").unwrap().is_none());
    // a file copied under another computer's name is not taken
    std::fs::copy(worktrees::file(&root, "rama", "studio"), worktrees::file(&root, "rama", "laptop")).unwrap();
    assert!(worktrees::read_for(&root, "rama", "laptop").unwrap().is_none());
    // a second repository, then the first one replaced
    let b = repo("/c/blog", vec![]);
    worktrees::upsert_for(&root, "rama", "studio", b, "t2", &exists).unwrap();
    let mut a2 = a.clone();
    a2.reclaimable = 99;
    let r = worktrees::upsert_for(&root, "rama", "studio", a2, "t3", &exists).unwrap();
    assert_eq!(r.repos.iter().map(|x| x.root.clone()).collect::<Vec<_>>(), [p("/c/blog"), p("/c/shop")]);
    assert_eq!(r.repos[1].reclaimable, 99);
    // repositories whose folder is gone drop out on the next write
    let only_shop = |x: &Path| x == p("/c/shop");
    let r = worktrees::upsert_for(&root, "rama", "studio", a, "t4", &only_shop).unwrap();
    assert_eq!(r.repos.len(), 1);
    worktrees::forget_for(&root, "rama", "studio", &p("/c/shop")).unwrap();
    assert!(worktrees::read_for(&root, "rama", "studio").unwrap().unwrap().repos.is_empty());
}

#[test]
fn readers_keep_only_safe_paths() {
    let mut w = wt("/c/shop-fix", vec![cache("/c/shop-fix/node_modules", CacheKind::NodeModules, 5), cache("/c/elsewhere/target", CacheKind::Target, 5), cache("/c/shop-fix", CacheKind::Target, 5), cache("/c/shop-fix/../x/target", CacheKind::Target, 5), cache("/c/shop-fix/t", CacheKind::CargoShared, 5)], vec![]);
    w.removable = true;
    w.blockers = vec![Blocker::Changes];
    let rel = Worktree { path: PathBuf::from("rel/wt"), ..wt("/x", vec![], vec![]) };
    let mut r = Report::new("rama", "studio");
    r.repos = vec![repo("/c/shop", vec![w, rel]), Repo { root: PathBuf::from("relative"), ..repo("/c/x", vec![]) }];
    let s = worktrees::sanitize(r);
    assert_eq!(s.repos.len(), 1);
    let w = &s.repos[0].worktrees;
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].caches.iter().map(|c| c.path.clone()).collect::<Vec<_>>(), [p("/c/shop-fix/node_modules")]);
    assert!(w[0].blockers.is_empty(), "removable carries no blockers");
}

#[test]
fn unknown_values_and_newer_schemas() {
    let text = r#"{"schema":1,"app":"rama","host":"studio","future":1,"repos":[{"root":"/c/shop","name":"shop","worktrees":[
        {"path":"/c/shop/wt","blockers":["somethingNew","changes"],"caches":[{"path":"/c/shop/wt/.venv","kind":"venv","bytes":3}]}]}]}"#;
    let text = if cfg!(windows) { text.replace("\"/c/", "\"C:/c/") } else { text.to_string() };
    let r = worktrees::parse(&text).unwrap();
    let w = &r.repos[0].worktrees[0];
    assert_eq!(w.blockers, [Blocker::Changes, Blocker::Other]);
    assert_eq!(w.caches[0].kind, CacheKind::Other);
    assert_eq!(w.bytes, None);
    assert!(matches!(worktrees::parse(r#"{"schema":2,"app":"rama","host":"h"}"#), Err(Error::NewerSchema(2))));
    assert!(matches!(worktrees::parse("{nope"), Err(Error::Parse(_))));
    // too large
    let dir = tempfile::tempdir().unwrap();
    let root = Root::at(dir.path());
    let f = worktrees::file(&root, "rama", "studio");
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(&f, vec![b' '; (worktrees::MAX_FILE + 1) as usize]).unwrap();
    assert!(matches!(worktrees::read_for(&root, "rama", "studio"), Err(Error::TooLarge)));
}

#[test]
fn sizes_add_up() {
    let w = wt("/c/a", vec![cache("/c/a/target", CacheKind::Target, 10), cache("/c/a/target/debug/incremental", CacheKind::Incremental, 4), cache("/c/a/node_modules", CacheKind::NodeModules, 3)], vec![]);
    assert_eq!(w.cache_bytes(), 13, "incremental is inside target: counted once");
    assert_eq!(repo("/c", vec![w.clone(), w]).bytes(), 200);
}
