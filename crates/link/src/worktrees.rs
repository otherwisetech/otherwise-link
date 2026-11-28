//! Worktrees and build caches: what a development app (rama) knows about the disk its
//! repositories use, for a disk app (diskmap) to show next to its own scan.
//!
//! One file per computer, `<root>/<app>/worktrees/<host>.json`, written only by that app on that
//! computer, whole and atomically ([`crate::fsutil::write_atomic`]). Paths only make sense on the
//! computer that measured them, and a data root may sync between computers, so readers take only
//! their own computer's file ([`read`]) and check its `host` too.
//!
//! ```json
//! { "schema": 1, "app": "rama", "host": "studio", "updatedAt": "2026-11-28T10:00:00Z",
//!   "repos": [ { "root": "/Users/ana/code/shop", "name": "shop", "updatedAt": "…",
//!     "defaultBranch": "origin/main", "reclaimable": 2147483648,
//!     "link": "otherwise://rama/reclaim?path=%2FUsers%2Fana%2Fcode%2Fshop",
//!     "worktrees": [ { "path": "/Users/ana/code/shop-fix", "branch": "fix/cart", "bytes": 3221225472,
//!       "touched": 1764300000, "removable": false, "blockers": ["unpushed"],
//!       "caches": [ { "path": "/Users/ana/code/shop-fix/target", "kind": "target",
//!                     "bytes": 3000000000, "offered": true } ] } ],
//!     "shared": [ { "path": "/Users/ana/.cache/shop-target", "kind": "cargoShared", "bytes": 9 } ] } ] }
//! ```
//!
//! The report is information, not permission: a reader that offers to delete something must
//! check it again itself (diskmap only deletes folders its own scan classified), and removing a
//! worktree is left to the writer (`link`, which opens the app's own confirmation).
use crate::fsutil::write_atomic;
use crate::paths::Root;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SCHEMA: u32 = 1;
/// The folder under `<root>/<app>/`.
pub const DIR: &str = "worktrees";
pub const MAX_FILE: u64 = 8 << 20;
pub const MAX_REPOS: usize = 200;
pub const MAX_WORKTREES: usize = 500;
pub const MAX_CACHES: usize = 200;

/// What a build or cache folder is. Unknown kinds read as [`CacheKind::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CacheKind {
    /// cargo / maven `target/`.
    Target,
    /// `incremental/` inside a cargo `target/`.
    Incremental,
    NodeModules,
    /// Next.js' `.next/`.
    Next,
    /// `dist/`, `build/`.
    BuildOutput,
    /// `.gradle/`.
    Gradle,
    /// Xcode's `DerivedData/`.
    DerivedData,
    /// A cargo target folder outside the repository that some worktrees build into.
    CargoShared,
    #[serde(other)]
    Other,
}

/// Why a worktree may not be removed. Unknown reasons read as [`Blocker::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Blocker {
    /// The repository's main worktree.
    Main,
    /// Open in the writer's window right now.
    Open,
    Locked,
    /// Uncommitted changes or untracked files.
    Changes,
    /// Its status couldn't be read (or its folder is missing).
    Unreadable,
    /// Commits its upstream doesn't have.
    Unpushed,
    /// Commits the default branch doesn't have.
    NotMerged,
    /// Its branch matches a protected pattern.
    ProtectedBranch,
    /// It lies in a place never touched (a tool's folder, a cloud folder, the system).
    ProtectedPlace,
    /// Its size isn't fully measured yet.
    NotMeasured,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cache {
    pub path: PathBuf,
    pub kind: CacheKind,
    pub bytes: u64,
    /// The writer would offer it in its own cleanup now (e.g. idle long enough).
    #[serde(default, skip_serializing_if = "is_false")]
    pub offered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub main: bool,
    /// Whole folder, build output included; `None` while not measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Last time someone worked in it (Unix seconds): newest file outside build folders or
    /// HEAD's commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touched: Option<i64>,
    /// The writer would offer to remove it (merged, clean, pushed, unlocked…).
    #[serde(default, skip_serializing_if = "is_false")]
    pub removable: bool,
    /// Why it can't be removed; empty when `removable`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<Blocker>,
    /// Build folders inside it, the outermost of each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caches: Vec<Cache>,
}

impl Worktree {
    pub fn cache_bytes(&self) -> u64 {
        self.caches.iter().filter(|c| c.kind != CacheKind::Incremental).map(|c| c.bytes).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    /// The main worktree's folder: the repository's identity in the file.
    pub root: PathBuf,
    pub name: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// Bytes the writer's own cleanup could free now.
    #[serde(default)]
    pub reclaimable: u64,
    /// Opens the writer's cleanup for this repository (`otherwise://rama/reclaim?path=…`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default)]
    pub worktrees: Vec<Worktree>,
    /// Build folders outside the repository its worktrees use (information only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<Cache>,
}

impl Repo {
    pub fn bytes(&self) -> u64 {
        self.worktrees.iter().filter_map(|w| w.bytes).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema: u32,
    pub app: String,
    pub host: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub repos: Vec<Repo>,
}

impl Report {
    pub fn new(app: &str, host: &str) -> Report {
        Report { schema: SCHEMA, app: app.to_string(), host: host.to_string(), updated_at: String::new(), repos: Vec::new() }
    }

    /// The repository and worktree `path` is (exactly).
    pub fn worktree(&self, path: &Path) -> Option<(&Repo, &Worktree)> {
        self.repos.iter().find_map(|r| r.worktrees.iter().find(|w| w.path == path).map(|w| (r, w)))
    }

    /// Every worktree folder, sorted.
    pub fn paths(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.repos.iter().flat_map(|r| r.worktrees.iter().map(|w| w.path.clone())).collect();
        v.sort();
        v.dedup();
        v
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// Written by a newer version of the library: not read.
    NewerSchema(u32),
    Parse(String),
    TooLarge,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::NewerSchema(n) => write!(f, "worktrees report schema {n} is newer than {SCHEMA}"),
            Error::Parse(e) => write!(f, "worktrees report: {e}"),
            Error::TooLarge => write!(f, "worktrees report larger than {MAX_FILE} bytes"),
        }
    }
}

impl std::error::Error for Error {}

/// A file-name-safe form of a computer name: ASCII letters and digits lowercased, runs of
/// anything else as one `-`, at most 64 characters; `unknown` if nothing is left.
pub fn host_key(host: &str) -> String {
    let mut out = String::new();
    for c in host.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let s: String = out.trim_matches('-').chars().take(64).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "unknown".into() } else { s }
}

/// `<root>/<app>/worktrees/<host key>.json`.
pub fn file(root: &Root, app: &str, host: &str) -> PathBuf {
    root.app_dir(app).join(DIR).join(format!("{}.json", host_key(host)))
}

fn plain_abs(p: &Path) -> bool {
    p.is_absolute() && p.components().all(|c| !matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir))
}

/// What a reader keeps: absolute paths without `..`, caches strictly inside their worktree,
/// within the limits, sorted (repositories and worktrees by path, caches by path).
pub fn sanitize(mut r: Report) -> Report {
    r.repos.retain(|x| plain_abs(&x.root));
    r.repos.truncate(MAX_REPOS);
    for repo in &mut r.repos {
        repo.worktrees.retain(|w| plain_abs(&w.path));
        repo.worktrees.truncate(MAX_WORKTREES);
        for w in &mut repo.worktrees {
            let root = w.path.clone();
            w.caches.retain(|c| plain_abs(&c.path) && c.path.starts_with(&root) && c.path != root && c.kind != CacheKind::CargoShared);
            w.caches.truncate(MAX_CACHES);
            w.caches.sort_by(|a, b| a.path.cmp(&b.path));
            if w.removable {
                w.blockers.clear();
            }
            w.blockers.sort();
            w.blockers.dedup();
        }
        repo.worktrees.sort_by(|a, b| b.main.cmp(&a.main).then_with(|| a.path.cmp(&b.path)));
        repo.shared.retain(|c| plain_abs(&c.path));
        repo.shared.truncate(MAX_CACHES);
        repo.shared.sort_by(|a, b| a.path.cmp(&b.path));
    }
    r.repos.sort_by(|a, b| a.root.cmp(&b.root));
    r.repos.dedup_by(|a, b| a.root == b.root);
    r
}

/// Parse a report's text (no host check).
pub fn parse(text: &str) -> Result<Report, Error> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
    let schema = v.get("schema").and_then(|s| s.as_u64()).ok_or_else(|| Error::Parse("no schema".into()))? as u32;
    if schema > SCHEMA {
        return Err(Error::NewerSchema(schema));
    }
    let r: Report = serde_json::from_value(v).map_err(|e| Error::Parse(e.to_string()))?;
    Ok(sanitize(r))
}

/// The report `app` wrote on the computer named `host`; `Ok(None)` when there is none, or the
/// file says it is another computer's.
pub fn read_for(root: &Root, app: &str, host: &str) -> Result<Option<Report>, Error> {
    let f = file(root, app, host);
    match std::fs::metadata(&f) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::Io(e)),
        Ok(m) if m.len() > MAX_FILE => return Err(Error::TooLarge),
        Ok(_) => {}
    }
    let text = std::fs::read_to_string(&f).map_err(Error::Io)?;
    let r = parse(&text)?;
    Ok((r.app == app && host_key(&r.host) == host_key(host)).then_some(r))
}

/// [`read_for`] this computer ([`crate::lock::host_name`]).
pub fn read(root: &Root, app: &str) -> Result<Option<Report>, Error> {
    read_for(root, app, crate::lock::host_name())
}

/// Writes the whole report (sanitized, `schema` set) for the computer it names.
pub fn write(root: &Root, report: &Report) -> std::io::Result<()> {
    let mut r = sanitize(report.clone());
    r.schema = SCHEMA;
    let text = serde_json::to_vec_pretty(&r).map_err(std::io::Error::other)?;
    write_atomic(&file(root, &r.app, &r.host), &text)
}

/// Replaces (or adds) one repository in this computer's report of `app` and drops the
/// repositories whose main folder is gone. `exists` decides that (the real app passes
/// `Path::is_dir`). Returns the report written.
pub fn upsert_for(root: &Root, app: &str, host: &str, repo: Repo, now: &str, exists: &dyn Fn(&Path) -> bool) -> std::io::Result<Report> {
    let mut r = read_for(root, app, host).ok().flatten().unwrap_or_else(|| Report::new(app, host));
    r.repos.retain(|x| x.root != repo.root && exists(&x.root));
    r.repos.push(repo);
    r.updated_at = now.to_string();
    r.host = host.to_string();
    write(root, &r)?;
    Ok(sanitize(r))
}

/// [`upsert_for`] this computer.
pub fn upsert(root: &Root, app: &str, repo: Repo, now: &str) -> std::io::Result<Report> {
    upsert_for(root, app, crate::lock::host_name(), repo, now, &|p: &Path| p.is_dir())
}

/// Removes one repository from this computer's report (nothing to do when absent).
pub fn forget_for(root: &Root, app: &str, host: &str, repo_root: &Path) -> std::io::Result<()> {
    let Some(mut r) = read_for(root, app, host).ok().flatten() else { return Ok(()) };
    let n = r.repos.len();
    r.repos.retain(|x| x.root != repo_root);
    if r.repos.len() == n {
        return Ok(());
    }
    write(root, &r)
}
