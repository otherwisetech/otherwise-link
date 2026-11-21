//! Changing where the data root is, and the one-time move from the pre-0.2 folder.
//!
//! **Changing the root** ([`plan`] → user confirms → [`relocate`]):
//!
//! 1. [`plan`] lists what is in the current root (everything but the pointer file, locks and
//!    temp files: `hub.json`, `entities.db`, `today/`, every app's folder) and what blocks it:
//!    the same name already at the destination, or a file open in another process / on another
//!    computer (its [`crate::lock`]). Show it to the user.
//! 2. [`relocate`] with [`Transfer::Move`] or [`Transfer::Copy`] refuses a blocked plan. It
//!    copies every item (databases through [`crate::storage::copy_sqlite`], verified; the
//!    registry last, while holding its write lock), and if anything fails removes what it copied
//!    and changes nothing. Then it writes the pointer, and for a move removes the originals
//!    (`hub.json` first, so apps still looking at the old root see the registry as off rather
//!    than writing to a file about to go). [`Transfer::Leave`] only writes the pointer: use a
//!    folder that already has the data (the same synced folder on a second computer).
//!
//! Apps keep working: they read the pointer on every [`crate::Root::system`]. An app whose own
//! database lives in `<root>/<app>/` and is open blocks the move (its lock) until it is closed.
//!
//! **Migration** ([`migrate_legacy`]): `hub.json`, `entities.db` and `today/*` from the old
//! platform folder are copied (verified) into the data root and then removed there; an item that
//! already exists at the root with other content is left where it was (reported, never
//! overwritten). A `.migrated` marker in the old folder makes it run once; an interrupted run
//! simply continues next time.
use crate::lock::{self, Holder, Lock, Scope};
use crate::paths::{Home, POINTER_FILE};
use crate::storage;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

pub const MIGRATED_MARKER: &str = ".migrated";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Move,
    Copy,
    /// Just point at the new folder (the data stays where it is).
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Database,
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    /// The destination already has something with this name.
    Exists,
    /// A file in it is open (another process here, or another computer).
    InUse(lock::LockInfo),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    /// Name under the root (`hub.json`, `vigia`…).
    pub name: String,
    pub kind: ItemKind,
    pub bytes: u64,
    pub files: usize,
    pub blocker: Option<Blocker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub from: PathBuf,
    pub to: PathBuf,
    pub items: Vec<PlanItem>,
}

impl Plan {
    pub fn blocked(&self) -> bool {
        self.items.iter().any(|i| i.blocker.is_some())
    }

    pub fn bytes(&self) -> u64 {
        self.items.iter().map(|i| i.bytes).sum()
    }
}

#[derive(Debug)]
pub enum Error {
    /// The new root is the current one.
    Same,
    /// The new root is inside something that would move.
    Inside(String),
    Blocked(Plan),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Same => write!(f, "that is already the data folder"),
            Error::Inside(n) => write!(f, "the new folder is inside {n}, which would move"),
            Error::Blocked(p) => write!(f, "{} item(s) can't move: already at the destination or in use", p.items.iter().filter(|i| i.blocker.is_some()).count()),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub from: PathBuf,
    pub to: PathBuf,
    pub transfer: Transfer,
    /// Items now at the new root.
    pub items: Vec<String>,
    /// Originals a move couldn't remove (the data is safe at the new root; tell the user).
    pub left_behind: Vec<PathBuf>,
}

fn ignored(name: &str) -> bool {
    name == POINTER_FILE || name == MIGRATED_MARKER || name == ".DS_Store" || name == "Thumbs.db" || name == "desktop.ini" || storage::skipped_in_copy(name)
}

fn walk_size(p: &Path, bytes: &mut u64, files: &mut usize, locks: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(p) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            walk_size(&e.path(), bytes, files, locks);
        } else if let Some(target) = name.strip_suffix(lock::SUFFIX) {
            locks.push(p.join(target));
        } else if !storage::skipped_in_copy(&name) {
            *bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            *files += 1;
        }
    }
}

fn in_use(targets: &[PathBuf]) -> Option<lock::LockInfo> {
    targets.iter().find_map(|t| match lock::inspect(t) {
        Holder::Here(i) | Holder::Elsewhere(i) => Some(i),
        _ => None,
    })
}

/// What a move from `from` to `to` would carry, and what stands in the way.
pub fn plan(from: &Path, to: &Path) -> Plan {
    let mut items = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(from).into_iter().flatten().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(t) = e.file_type() else { continue };
        if ignored(&name) || !(t.is_dir() || t.is_file()) {
            continue;
        }
        let p = e.path();
        let (mut bytes, mut files, mut locks) = (0, 0, Vec::new());
        let kind = if t.is_dir() {
            walk_size(&p, &mut bytes, &mut files, &mut locks);
            ItemKind::Folder
        } else {
            bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
            files = 1;
            locks.push(p.clone());
            if storage::is_sqlite(&p) { ItemKind::Database } else { ItemKind::File }
        };
        let blocker = if to.join(&name).exists() { Some(Blocker::Exists) } else { in_use(&locks).map(Blocker::InUse) };
        items.push(PlanItem { name, kind, bytes, files, blocker });
    }
    // shared settings first, the registry last (see the module docs)
    items.sort_by_key(|i| match i.name.as_str() {
        "hub.json" => 0,
        "entities.db" => 2,
        _ => 1,
    });
    Plan { from: from.to_path_buf(), to: to.to_path_buf(), items }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Moves / copies the data root of `home` to `to`, or only points there; see the module docs.
pub fn relocate(home: &Home, to: &Path, transfer: Transfer) -> Result<Report, Error> {
    let from = home.data_root();
    if same_path(&from, to) {
        return Err(Error::Same);
    }
    if transfer == Transfer::Leave {
        std::fs::create_dir_all(to)?;
        home.set_pointer(to)?;
        return Ok(Report { from, to: to.to_path_buf(), transfer, items: Vec::new(), left_behind: Vec::new() });
    }
    let p = plan(&from, to);
    if let Some(i) = p.items.iter().find(|i| to.starts_with(from.join(&i.name))) {
        return Err(Error::Inside(i.name.clone()));
    }
    if p.blocked() {
        return Err(Error::Blocked(p));
    }
    std::fs::create_dir_all(to)?;
    let mut made: Vec<PathBuf> = Vec::new();
    let mut guard = None;
    let res: io::Result<()> = (|| {
        for i in &p.items {
            let (src, dest) = (from.join(&i.name), to.join(&i.name));
            match i.kind {
                ItemKind::Folder => {
                    made.push(dest.clone());
                    storage::copy_tree(&src, &dest)?;
                }
                ItemKind::Database => {
                    if i.name == "entities.db" {
                        // hold the registry's write lock: other apps wait (busy timeout) instead of
                        // writing to the copy we're leaving
                        let c = rusqlite::Connection::open(&src).map_err(io::Error::other)?;
                        c.busy_timeout(std::time::Duration::from_secs(10)).map_err(io::Error::other)?;
                        c.execute_batch("BEGIN IMMEDIATE").map_err(io::Error::other)?;
                        guard = Some(c);
                    }
                    storage::copy_sqlite(&src, &dest)?;
                    made.push(dest);
                }
                ItemKind::File => {
                    storage::copy_file(&src, &dest)?;
                    made.push(dest);
                }
            }
        }
        home.set_pointer(to)
    })();
    if let Some(c) = guard.take() {
        let _ = c.execute_batch("ROLLBACK");
    }
    if let Err(e) = res {
        for m in made.iter().rev() {
            if m.is_dir() {
                let _ = std::fs::remove_dir_all(m);
            } else {
                let _ = storage::remove_sqlite(m).or_else(|_| std::fs::remove_file(m));
            }
        }
        return Err(Error::Io(e));
    }
    let mut left_behind = Vec::new();
    if transfer == Transfer::Move {
        for i in &p.items {
            let src = from.join(&i.name);
            let r = match i.kind {
                ItemKind::Folder => std::fs::remove_dir_all(&src),
                ItemKind::Database => storage::remove_sqlite(&src),
                ItemKind::File => std::fs::remove_file(&src),
            };
            if r.is_err() {
                left_behind.push(src);
            }
        }
    }
    Ok(Report { from, to: to.to_path_buf(), transfer, items: p.items.into_iter().map(|i| i.name).collect(), left_behind })
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Marker {
    pub schema: u32,
    pub to: PathBuf,
    pub at: i64,
    pub moved: Vec<String>,
    /// Left here because the data root already had a different one.
    pub kept: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// No old folder, or it is the data root.
    Nothing,
    AlreadyDone,
    /// Another process is migrating right now.
    Busy,
    Done(Marker),
}

fn same_content(a: &Path, b: &Path) -> io::Result<bool> {
    if storage::is_sqlite(a) && storage::is_sqlite(b) {
        return Ok(storage::verify_sqlite(a)? == storage::verify_sqlite(b)?);
    }
    Ok(std::fs::read(a)? == std::fs::read(b)?)
}

fn migrate_file(src: &Path, dest: &Path, name: String, m: &mut Marker) -> io::Result<()> {
    if dest.exists() {
        if same_content(src, dest)? {
            storage::remove_sqlite(src)?;
            m.moved.push(name);
        } else {
            m.kept.push(name);
        }
        return Ok(());
    }
    storage::copy_any(src, dest)?;
    storage::remove_sqlite(src)?;
    m.moved.push(name);
    Ok(())
}

/// Moves the pre-0.2 shared files from `legacy` into `root`; see the module docs.
pub fn migrate_legacy(legacy: &Path, root: &Path) -> io::Result<Migration> {
    if !legacy.is_dir() || same_path(legacy, root) {
        return Ok(Migration::Nothing);
    }
    if legacy.join(MIGRATED_MARKER).exists() {
        return Ok(Migration::AlreadyDone);
    }
    let names = ["hub.json", "entities.db", "today"];
    if !names.iter().any(|n| legacy.join(n).exists()) {
        return Ok(Migration::Nothing);
    }
    let Ok(_guard) = Lock::acquire(&legacy.join(".migrating"), "otherwise-link", Scope::Process) else { return Ok(Migration::Busy) };
    let mut m = Marker { schema: 1, to: root.to_path_buf(), at: lock::now(), ..Default::default() };
    // shared settings last: until it moves, old apps keep using the old folder consistently
    for n in ["entities.db", "today", "hub.json"] {
        let src = legacy.join(n);
        if n == "today" {
            if !src.is_dir() {
                continue;
            }
            let mut files: Vec<_> = std::fs::read_dir(&src)?.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).collect();
            files.sort_by_key(|e| e.file_name());
            for e in files {
                let fname = e.file_name().to_string_lossy().into_owned();
                if storage::skipped_in_copy(&fname) {
                    continue;
                }
                migrate_file(&e.path(), &root.join("today").join(&fname), format!("today/{fname}"), &mut m)?;
            }
            let _ = std::fs::remove_dir(&src);
        } else if src.is_file() {
            migrate_file(&src, &root.join(n), n.to_string(), &mut m)?;
        }
    }
    crate::fsutil::write_atomic(&legacy.join(MIGRATED_MARKER), (serde_json::to_string_pretty(&m).unwrap_or_default() + "\n").as_bytes())?;
    Ok(Migration::Done(m))
}
