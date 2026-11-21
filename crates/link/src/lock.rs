//! "Is this file open somewhere else?" A small JSON file next to the target, `<file>.lock.json`:
//!
//! ```json
//! { "schema": 1, "app": "vigia", "host": "Juan's MacBook Pro", "pid": 4242,
//!   "scope": "process", "since": 1763700000, "heartbeat": 1763700060 }
//! ```
//!
//! The holder rewrites `heartbeat` every [`HEARTBEAT_EVERY`]; a lock whose heartbeat is older
//! than [`STALE_AFTER`] (the holder crashed, or the computer went to sleep) is free to take, as
//! is one from this computer whose process is gone. A fresh lock from **another computer** means
//! the file is open there: apps show "open on <host>" and offer to open it read-only or take it
//! over ([`Lock::take_over`]) once the user confirms.
//!
//! [`Scope::Computer`] locks are shared by every process on one computer (the entity registry,
//! which several apps open at once): they only keep other computers out.
//!
//! This guards against two computers writing one synced file at the same time; it isn't sync.
//! The sync client still decides which version wins, so after switching computers wait for it to
//! finish (and check [`crate::cloud::conflicted_copies`]).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

pub const SUFFIX: &str = ".lock.json";
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(60);
/// Long enough for a sync client to deliver a heartbeat from the other computer.
pub const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// One process (a document window).
    #[default]
    Process,
    /// Any process of this computer.
    Computer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LockInfo {
    pub schema: u32,
    pub app: String,
    pub host: String,
    pub pid: u32,
    pub scope: Scope,
    /// Unix seconds.
    pub since: i64,
    pub heartbeat: i64,
}

impl Default for LockInfo {
    fn default() -> Self {
        LockInfo { schema: 1, app: String::new(), host: String::new(), pid: 0, scope: Scope::Process, since: 0, heartbeat: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    Free,
    /// This process (or, for a computer lock, this computer).
    Mine(LockInfo),
    /// Left behind (old heartbeat, or a dead process on this computer): free to take.
    Stale(LockInfo),
    /// Another live process on this computer.
    Here(LockInfo),
    /// Another computer, recently.
    Elsewhere(LockInfo),
}

impl Holder {
    /// Free, ours, or stale.
    pub fn is_available(&self) -> bool {
        matches!(self, Holder::Free | Holder::Mine(_) | Holder::Stale(_))
    }

    pub fn info(&self) -> Option<&LockInfo> {
        match self {
            Holder::Free => None,
            Holder::Mine(i) | Holder::Stale(i) | Holder::Here(i) | Holder::Elsewhere(i) => Some(i),
        }
    }
}

/// Who and when, injectable for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    pub host: String,
    pub pid: u32,
}

impl Me {
    pub fn current() -> Me {
        Me { host: host_name().to_string(), pid: std::process::id() }
    }
}

/// A readable name of this computer (macOS: the Sharing name; else `COMPUTERNAME` /
/// `hostname`).
pub fn host_name() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| {
        let run = |cmd: &str, args: &[&str]| {
            std::process::Command::new(cmd).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
        };
        let mac = if cfg!(target_os = "macos") { run("scutil", &["--get", "ComputerName"]) } else { None };
        mac.or_else(|| std::env::var("COMPUTERNAME").ok().filter(|s| !s.is_empty()))
            .or_else(|| run("hostname", &[]))
            .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "unknown".into())
    })
}

/// Is a process with this id running on this computer? Unknown counts as running.
pub fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    if cfg!(target_os = "linux") {
        return Path::new("/proc").join(pid.to_string()).exists();
    }
    if cfg!(windows) {
        return match std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"]).output() {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")),
            Err(_) => true,
        };
    }
    match std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status() {
        Ok(s) => s.success(),
        Err(_) => true,
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `<target>.lock.json`.
pub fn lock_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(SUFFIX);
    PathBuf::from(s)
}

pub fn read(target: &Path) -> Option<LockInfo> {
    std::fs::read_to_string(lock_path(target)).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Classifies a lock as seen by `me` at `now`.
pub fn classify(info: LockInfo, me: &Me, now: i64, alive: impl Fn(u32) -> bool) -> Holder {
    let fresh = now - info.heartbeat < STALE_AFTER.as_secs() as i64;
    if info.host != me.host {
        return if fresh { Holder::Elsewhere(info) } else { Holder::Stale(info) };
    }
    if info.scope == Scope::Computer || info.pid == me.pid {
        return Holder::Mine(info);
    }
    if fresh && alive(info.pid) { Holder::Here(info) } else { Holder::Stale(info) }
}

pub fn inspect_as(target: &Path, me: &Me, now: i64, alive: impl Fn(u32) -> bool) -> Holder {
    match read(target) {
        None => Holder::Free,
        Some(i) => classify(i, me, now, alive),
    }
}

/// Who holds `target`'s lock, from this process's point of view.
pub fn inspect(target: &Path) -> Holder {
    inspect_as(target, &Me::current(), now(), process_alive)
}

/// A held lock; released (the file removed) on drop, except a computer lock, which other
/// processes here may share and which just goes stale.
#[derive(Debug)]
pub struct Lock {
    target: PathBuf,
    info: LockInfo,
    me: Me,
    released: bool,
}

impl Lock {
    /// Takes the lock unless a live process here or another computer holds it (returned).
    pub fn acquire(target: &Path, app: &str, scope: Scope) -> Result<Lock, Holder> {
        Lock::acquire_as(target, app, scope, Me::current(), now(), process_alive)
    }

    pub fn acquire_as(target: &Path, app: &str, scope: Scope, me: Me, now: i64, alive: impl Fn(u32) -> bool) -> Result<Lock, Holder> {
        let h = inspect_as(target, &me, now, alive);
        if !h.is_available() {
            return Err(h);
        }
        let since = match &h {
            Holder::Mine(i) => i.since,
            _ => now,
        };
        let mut l = Lock { target: target.to_path_buf(), info: LockInfo { schema: 1, app: app.into(), host: me.host.clone(), pid: me.pid, scope, since, heartbeat: now }, me, released: false };
        if let Holder::Mine(i) = &h
            && scope == Scope::Computer
            && now - i.heartbeat < HEARTBEAT_EVERY.as_secs() as i64
        {
            // shared and fresh enough: don't churn the synced folder
            l.info = i.clone();
            return Ok(l);
        }
        l.write().map_err(|_| h)?;
        Ok(l)
    }

    /// Takes the lock whoever holds it (after the user confirmed "use it here anyway").
    pub fn take_over(target: &Path, app: &str, scope: Scope) -> std::io::Result<Lock> {
        let me = Me::current();
        let t = now();
        let mut l = Lock { target: target.to_path_buf(), info: LockInfo { schema: 1, app: app.into(), host: me.host.clone(), pid: me.pid, scope, since: t, heartbeat: t }, me, released: false };
        l.write()?;
        Ok(l)
    }

    fn write(&mut self) -> std::io::Result<()> {
        let s = serde_json::to_string_pretty(&self.info).unwrap_or_default() + "\n";
        crate::fsutil::write_atomic(&lock_path(&self.target), s.as_bytes())
    }

    pub fn info(&self) -> &LockInfo {
        &self.info
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Refreshes the heartbeat. Err when someone else took the lock in the meantime (another
    /// computer took over): the app should stop writing and tell the user.
    pub fn heartbeat(&mut self) -> Result<(), Holder> {
        self.heartbeat_at(now())
    }

    pub fn heartbeat_at(&mut self, now: i64) -> Result<(), Holder> {
        if let Some(i) = read(&self.target)
            && !(i.host == self.me.host && (i.pid == self.me.pid || self.info.scope == Scope::Computer))
        {
            return Err(classify(i, &self.me, now, |_| true));
        }
        self.info.heartbeat = now;
        self.write().map_err(|_| Holder::Free)
    }

    /// Removes the lock file if it is still ours.
    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        if self.info.scope == Scope::Computer {
            return;
        }
        if read(&self.target).is_some_and(|i| i.host == self.me.host && i.pid == self.me.pid) {
            let _ = std::fs::remove_file(lock_path(&self.target));
        }
    }

    /// Forget the lock without removing it (it moved with the file, or the process hands it on).
    pub fn forget(mut self) {
        self.released = true;
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.release_inner();
    }
}
