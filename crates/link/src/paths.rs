//! Where Otherwise apps keep their data: one **data root** per user, chosen by the user (a local
//! folder, OneDrive, Dropbox…), default `~/.otherwise` (`%USERPROFILE%\.otherwise` on Windows).
//!
//! The choice lives in a tiny pointer file at a fixed place, `~/.otherwise/config.json`:
//!
//! ```json
//! { "schema": 1, "data_root": "/Users/me/Library/CloudStorage/OneDrive-Personal/Otherwise" }
//! ```
//!
//! Missing, unreadable or without `data_root`: the default. `OTHERWISE_DATA_DIR` overrides both
//! (tests, portable setups, a second profile). Under the root:
//!
//! ```text
//! <root>/
//!   hub.json          shared settings (settings.rs)
//!   entities.db       shared entity registry, only once the user opts in (registry.rs)
//!   today/<app>.json  one Today feed per app (today.rs)
//!   <app>/            each app's default folder for its own data (vigia/vigia.db, nodo/…)
//! ```
//!
//! Versions before 0.2 used a platform folder ([`Home::legacy_root`]); [`Root::system`] moves its
//! contents over once (see [`crate::relocate::migrate_legacy`]).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const ENV: &str = "OTHERWISE_DATA_DIR";
/// Folder of the pointer file (and of the default root), under the home folder.
pub const POINTER_DIR: &str = ".otherwise";
pub const POINTER_FILE: &str = "config.json";

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn home() -> PathBuf {
    let order: [&str; 2] = if cfg!(windows) { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    order.iter().find_map(|v| env_dir(v)).unwrap_or_else(|| PathBuf::from("."))
}

/// The pre-0.2 shared folder of this platform.
fn system_legacy_root(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("Otherwise")
    } else if cfg!(windows) {
        env_dir("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming")).join("Otherwise")
    } else {
        env_dir("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share")).join("otherwise")
    }
}

/// `~/.otherwise/config.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pointer {
    pub schema: u32,
    /// The chosen data root; absent: the default (`~/.otherwise`).
    #[serde(alias = "dataRoot", skip_serializing_if = "Option::is_none")]
    pub data_root: Option<PathBuf>,
}

/// Folders an older helper (Otherwise Apps 0.6.0) left in the default root that belong to the
/// device, not the data: never moved with the root ([`crate::relocate::plan`]).
pub const DEVICE_LOCAL_IN_ROOT: &[&str] = &["helper"];

/// This computer's own folder for state that must never sync or move with the data root
/// (background-job runs, backoff, pause, which device this is):
///
/// - macOS: `~/Library/Application Support/com.otherwisetech.device/`
/// - Windows: `%LOCALAPPDATA%\Otherwise\device\`
/// - Linux: `$XDG_STATE_HOME/otherwise/` (default `~/.local/state/otherwise/`)
///
/// Deliberately not the pre-0.2 shared folder ([`Home::legacy_root`]), which is migrated into
/// the root. Not created here.
fn system_device_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("com.otherwisetech.device")
    } else if cfg!(windows) {
        env_dir("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local")).join("Otherwise").join("device")
    } else {
        env_dir("XDG_STATE_HOME").unwrap_or_else(|| home.join(".local").join("state")).join("otherwise")
    }
}

/// See [`system_device_dir`]; `OTHERWISE_DEVICE_DIR` overrides it (tests, a second profile).
pub fn device_dir() -> PathBuf {
    env_dir(DEVICE_ENV).unwrap_or_else(|| system_device_dir(&home()))
}

pub const DEVICE_ENV: &str = "OTHERWISE_DEVICE_DIR";

/// A user's home as far as data placement goes; [`Home::system`] for the real one, [`Home::at`]
/// for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    pub home: PathBuf,
    /// Where versions before 0.2 kept the shared files.
    pub legacy: PathBuf,
    /// This computer's own folder ([`device_dir`]).
    pub device: PathBuf,
}

impl Home {
    pub fn system() -> Home {
        let home = home();
        Home { legacy: system_legacy_root(&home), device: device_dir(), home }
    }

    /// A test home: the legacy folder is `<dir>/legacy`, the device folder `<dir>/device`.
    pub fn at(dir: impl Into<PathBuf>) -> Home {
        let home = dir.into();
        Home { legacy: home.join("legacy"), device: home.join("device"), home }
    }

    pub fn pointer_dir(&self) -> PathBuf {
        self.home.join(POINTER_DIR)
    }

    pub fn pointer_file(&self) -> PathBuf {
        self.pointer_dir().join(POINTER_FILE)
    }

    /// `~/.otherwise`.
    pub fn default_root(&self) -> PathBuf {
        self.pointer_dir()
    }

    pub fn legacy_root(&self) -> &Path {
        &self.legacy
    }

    pub fn pointer(&self) -> Pointer {
        std::fs::read_to_string(self.pointer_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// The pointer's root if it names an absolute folder, else the default.
    pub fn data_root(&self) -> PathBuf {
        self.pointer().data_root.filter(|p| p.is_absolute()).unwrap_or_else(|| self.default_root())
    }

    /// Points at `root` (the default root is stored as "no choice").
    pub fn set_pointer(&self, root: &Path) -> std::io::Result<()> {
        let data_root = (root != self.default_root()).then(|| root.to_path_buf());
        let p = Pointer { schema: 1, data_root };
        crate::fsutil::write_atomic(&self.pointer_file(), (serde_json::to_string_pretty(&p).unwrap_or_default() + "\n").as_bytes())
    }
}

/// The data root of this user on this machine (`OTHERWISE_DATA_DIR`, else the pointer file, else
/// `~/.otherwise`). Read fresh on every call: another app may have moved it.
pub fn data_root() -> PathBuf {
    env_dir(ENV).unwrap_or_else(|| Home::system().data_root())
}

/// `<data root>/<app>`: the default folder for an app's own files (its database, attachments).
/// Not created here.
pub fn app_dir(app: &str) -> PathBuf {
    data_root().join(app)
}

/// Moves (`move_data`) or just re-points the data root to `new`; see [`crate::relocate`] for the
/// plan / confirmation step and the details.
pub fn set_data_root(new: &Path, move_data: bool) -> Result<crate::relocate::Report, crate::relocate::Error> {
    let t = if move_data { crate::relocate::Transfer::Move } else { crate::relocate::Transfer::Leave };
    crate::relocate::relocate(&Home::system(), new, t)
}

/// Moves the pre-0.2 folder's files to the data root, once per process (cheap after the first
/// time: a marker file). Skipped when `OTHERWISE_DATA_DIR` is set.
pub fn ensure_migrated() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        if env_dir(ENV).is_some() {
            return;
        }
        let home = Home::system();
        if let Err(e) = crate::relocate::migrate_legacy(home.legacy_root(), &home.data_root()) {
            eprintln!("otherwise-link: moving {} to the data root failed: {e}", home.legacy_root().display());
        }
    });
}

/// A data root; [`Root::system`] for the real one, [`Root::at`] for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root(pub PathBuf);

impl Root {
    /// The user's data root (after moving the pre-0.2 files over, the first time).
    pub fn system() -> Root {
        ensure_migrated();
        Root(data_root())
    }

    pub fn at(dir: impl Into<PathBuf>) -> Root {
        Root(dir.into())
    }

    pub fn dir(&self) -> &Path {
        &self.0
    }

    pub fn settings_file(&self) -> PathBuf {
        self.0.join("hub.json")
    }

    pub fn entities_db(&self) -> PathBuf {
        self.0.join("entities.db")
    }

    pub fn today_dir(&self) -> PathBuf {
        self.0.join("today")
    }

    /// `<root>/<app>`.
    pub fn app_dir(&self, app: &str) -> PathBuf {
        self.0.join(app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_defaults_and_roundtrip() {
        let t = tempfile::tempdir().unwrap();
        let h = Home::at(t.path());
        assert_eq!(h.data_root(), t.path().join(".otherwise"));
        let other = t.path().join("OneDrive").join("Otherwise");
        h.set_pointer(&other).unwrap();
        assert_eq!(h.data_root(), other);
        let raw = std::fs::read_to_string(h.pointer_file()).unwrap();
        assert!(raw.contains("\"data_root\""), "{raw}");
        h.set_pointer(&h.default_root()).unwrap();
        assert_eq!(h.pointer().data_root, None);
        assert_eq!(h.data_root(), h.default_root());
        // garbage and relative paths fall back to the default
        std::fs::write(h.pointer_file(), "{nope").unwrap();
        assert_eq!(h.data_root(), h.default_root());
        std::fs::write(h.pointer_file(), r#"{"dataRoot":"relative/x"}"#).unwrap();
        assert_eq!(h.data_root(), h.default_root());
        assert_eq!(Root::at(&other).app_dir("vigia"), other.join("vigia"));
    }

    #[test]
    fn device_dir_is_outside_the_roots() {
        let h = Home::system();
        let dev = system_device_dir(&h.home);
        assert!(!dev.starts_with(h.default_root()), "{dev:?} would move with the default root");
        assert!(!dev.starts_with(&h.legacy) && !h.legacy.starts_with(&dev), "{dev:?} vs the pre-0.2 folder {:?}", h.legacy);
        assert_ne!(dev, h.default_root());
    }
}
