//! Moving and copying the user's files without ever losing them, and keeping SQLite safe in a
//! synced folder. Every app uses these for "Save As…", "Move to…" and the data-root move.
//!
//! - [`copy_sqlite`]: `VACUUM INTO` a temp file next to the destination (a consistent snapshot
//!   even while other connections write, WAL included), then **verify** it: reopen,
//!   `integrity_check`, same `application_id` / `user_version`, and the same content
//!   ([`fingerprint`]: every row of every table) as the source; only then rename it into place.
//!   A write that lands between the snapshot and the comparison makes it retry.
//! - [`move_sqlite`]: copy + verify, then delete the original (and its `-journal` / `-wal` /
//!   `-shm`). The caller must have closed its own connection.
//! - [`copy_file`] / [`copy_tree`] / [`move_tree`]: the same for plain files and folders
//!   (byte-compared; SQLite files inside a folder go through [`copy_sqlite`]; lock files and
//!   SQLite sidecars are skipped). A move within one volume is a plain rename.
//! - [`ensure_rollback_journal`]: `journal_mode=DELETE`, never WAL (WAL's `-shm` index and the
//!   separate `-wal` file don't survive sync clients).
//! - [`status`]: cloud folder, lock holder and conflicted copies of a file, for the UI.
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

/// SQLite's sidecar files, which belong to the database and are never copied on their own.
pub const SIDECARS: &[&str] = &["-journal", "-wal", "-shm"];

fn sql_err(e: rusqlite::Error) -> io::Error {
    io::Error::other(e)
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

fn tmp_next_to(dest: &Path, tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = dest.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    dir.join(format!(".{name}.{tag}-{}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// Does the file start with the SQLite header?
pub fn is_sqlite(path: &Path) -> bool {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut b)).is_ok() && &b == b"SQLite format 3\0"
}

/// Switches a connection to the rollback journal if it isn't (setting the mode takes a lock, so
/// only when it differs).
pub fn ensure_rollback_journal(conn: &Connection) -> rusqlite::Result<()> {
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("delete") {
        let _: String = conn.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
    }
    Ok(())
}

/// What a database holds, for comparing a copy with its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub application_id: i64,
    pub user_version: i64,
    /// (table, rows, order-independent hash of the rows)
    pub tables: Vec<(String, u64, u64)>,
}

/// Reads the whole database inside one read transaction.
pub fn fingerprint(conn: &Connection) -> rusqlite::Result<Fingerprint> {
    conn.execute_batch("BEGIN")?;
    let r = (|| {
        let application_id = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        let user_version = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let mut names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        names.sort();
        let mut tables = Vec::new();
        for t in names {
            let mut st = conn.prepare(&format!("SELECT * FROM \"{}\"", t.replace('"', "\"\"")))?;
            let cols = st.column_count();
            let mut rows = st.query([])?;
            let (mut n, mut sum) = (0u64, 0u64);
            while let Some(row) = rows.next()? {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for i in 0..cols {
                    match row.get_ref(i)? {
                        ValueRef::Null => 0u8.hash(&mut h),
                        ValueRef::Integer(v) => (1u8, v).hash(&mut h),
                        ValueRef::Real(v) => (2u8, v.to_bits()).hash(&mut h),
                        ValueRef::Text(v) => (3u8, v).hash(&mut h),
                        ValueRef::Blob(v) => (4u8, v).hash(&mut h),
                    }
                }
                n += 1;
                sum = sum.wrapping_add(h.finish());
            }
            tables.push((t, n, sum));
        }
        Ok(Fingerprint { application_id, user_version, tables })
    })();
    let _ = conn.execute_batch("COMMIT");
    r
}

/// Opens a database read-only and checks it: `integrity_check` must say `ok`.
pub fn verify_sqlite(path: &Path) -> io::Result<Fingerprint> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(sql_err)?;
    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(sql_err)?;
    if check != "ok" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("integrity check of {} failed: {check}", path.display())));
    }
    fingerprint(&conn).map_err(sql_err)
}

fn sync_file(p: &Path) -> io::Result<()> {
    std::fs::OpenOptions::new().read(true).write(true).open(p)?.sync_all()
}

/// Copies the database `src` to `dest` (which must not exist) and verifies the copy; see the
/// module docs. The copy uses the rollback journal.
pub fn copy_sqlite(src: &Path, dest: &Path) -> io::Result<Fingerprint> {
    if dest.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", dest.display())));
    }
    if let Some(d) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let conn = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .or_else(|_| Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX))
        .map_err(sql_err)?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(sql_err)?;
    let mut last = None;
    for _ in 0..5 {
        let tmp = tmp_next_to(dest, "copy");
        let res = (|| {
            conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()]).map_err(sql_err)?;
            {
                let c = Connection::open(&tmp).map_err(sql_err)?;
                ensure_rollback_journal(&c).map_err(sql_err)?;
            }
            sync_file(&tmp)?;
            let got = verify_sqlite(&tmp)?;
            let want = fingerprint(&conn).map_err(sql_err)?;
            if got != want {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "the database changed while it was copied"));
            }
            Ok(got)
        })();
        match res {
            Ok(fp) => {
                if dest.exists() {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", dest.display())));
                }
                std::fs::rename(&tmp, dest).inspect_err(|_| {
                    let _ = std::fs::remove_file(&tmp);
                })?;
                return Ok(fp);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                let _ = std::fs::remove_file(with_suffix(&tmp, "-journal"));
                if e.kind() != io::ErrorKind::Interrupted {
                    return Err(e);
                }
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("copy failed")))
}

/// Removes a database file and its sidecars.
pub fn remove_sqlite(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)?;
    for s in SIDECARS {
        let _ = std::fs::remove_file(with_suffix(path, s));
    }
    Ok(())
}

/// [`copy_sqlite`], then removes the original. Close your own connection to `src` first.
pub fn move_sqlite(src: &Path, dest: &Path) -> io::Result<Fingerprint> {
    let fp = copy_sqlite(src, dest)?;
    // one last look: the copy is what we'll keep
    if verify_sqlite(dest)? != fp {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the copy changed after it was verified"));
    }
    remove_sqlite(src)?;
    Ok(fp)
}

fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    use std::io::Read;
    let (fa, fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let (mut ra, mut rb) = (io::BufReader::new(fa), io::BufReader::new(fb));
    let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let n = ra.read(&mut ba)?;
        if n == 0 {
            return Ok(rb.read(&mut bb)? == 0);
        }
        rb.read_exact(&mut bb[..n])?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

/// Copies a plain file to `dest` (which must not exist) through a temp file, flushes it,
/// compares the bytes, then renames it into place.
pub fn copy_file(src: &Path, dest: &Path) -> io::Result<()> {
    if dest.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", dest.display())));
    }
    if let Some(d) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let tmp = tmp_next_to(dest, "copy");
    let res = (|| {
        std::fs::copy(src, &tmp)?;
        sync_file(&tmp)?;
        if !same_bytes(src, &tmp)? {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("copy of {} differs", src.display())));
        }
        std::fs::rename(&tmp, dest)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Files a folder copy leaves out: SQLite sidecars (the database is copied whole), lock files
/// (they belong to the old place) and our own temp files.
pub fn skipped_in_copy(name: &str) -> bool {
    SIDECARS.iter().any(|s| name.ends_with(s)) || name.ends_with(crate::lock::SUFFIX) || (name.starts_with('.') && name.ends_with(".tmp"))
}

/// Copies a file: SQLite databases with [`copy_sqlite`], anything else with [`copy_file`].
pub fn copy_any(src: &Path, dest: &Path) -> io::Result<()> {
    if is_sqlite(src) { copy_sqlite(src, dest).map(|_| ()) } else { copy_file(src, dest) }
}

/// Copies the folder `src` into `dest` (created; existing files there are an error, nothing is
/// overwritten). Returns the files copied (destination paths).
pub fn copy_tree(src: &Path, dest: &Path) -> io::Result<Vec<PathBuf>> {
    let mut done = Vec::new();
    let r = copy_tree_into(src, dest, &mut done);
    if r.is_err() {
        for f in done.iter().rev() {
            let _ = std::fs::remove_file(f);
        }
    }
    r.map(|_| done)
}

fn copy_tree_into(src: &Path, dest: &Path, done: &mut Vec<PathBuf>) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut entries: Vec<_> = std::fs::read_dir(src)?.collect::<io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name();
        let (from, to) = (e.path(), dest.join(&name));
        let t = e.file_type()?;
        if t.is_dir() {
            copy_tree_into(&from, &to, done)?;
        } else if t.is_file() && !skipped_in_copy(&name.to_string_lossy()) {
            copy_any(&from, &to)?;
            done.push(to);
        }
    }
    Ok(())
}

/// Removes what [`copy_tree`] copied from `src` (files it skipped, like locks, stay) and the
/// folders left empty.
fn remove_copied(src: &Path) -> io::Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            remove_copied(&p)?;
        } else {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(crate::lock::SUFFIX) {
                std::fs::remove_file(&p)?;
            }
        }
    }
    let _ = std::fs::remove_dir(src);
    Ok(())
}

/// Moves a folder: a rename when `dest` doesn't exist and both are on one volume, else
/// [`copy_tree`] (verified) and then removing the originals.
pub fn move_tree(src: &Path, dest: &Path) -> io::Result<()> {
    if !dest.exists() {
        if let Some(d) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d)?;
        }
        if std::fs::rename(src, dest).is_ok() {
            return Ok(());
        }
    }
    copy_tree(src, dest)?;
    remove_copied(src)
}

/// Moves a single file of any kind (rename when possible, else verified copy + delete).
pub fn move_any(src: &Path, dest: &Path) -> io::Result<()> {
    if dest.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", dest.display())));
    }
    if let Some(d) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let sqlite = is_sqlite(src);
    // a database with a live journal / WAL isn't one file yet: go through the snapshot copy
    let sidecar = SIDECARS.iter().any(|s| with_suffix(src, s).exists());
    if !(sqlite && sidecar) && std::fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    if sqlite {
        move_sqlite(src, dest).map(|_| ())
    } else {
        copy_file(src, dest)?;
        std::fs::remove_file(src)
    }
}

/// How a file sits in its folder, for the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub cloud: Option<crate::cloud::CloudFolder>,
    pub lock: crate::lock::Holder,
    pub conflicts: Vec<PathBuf>,
}

impl Status {
    /// Something the user should look at: open on another computer, or conflicted copies.
    pub fn needs_attention(&self) -> bool {
        matches!(self.lock, crate::lock::Holder::Elsewhere(_)) || !self.conflicts.is_empty()
    }
}

pub fn status(file: &Path) -> Status {
    let cloud = crate::cloud::detect(file);
    let conflicts = crate::cloud::conflicted_copies_with(file, cloud.as_ref().map(|c| c.provider));
    Status { cloud, lock: crate::lock::inspect(file), conflicts }
}
