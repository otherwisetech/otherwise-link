//! Where the shared files live. One folder per user, `Otherwise`:
//!
//! - macOS: `~/Library/Application Support/Otherwise`
//! - Windows: `%APPDATA%\Otherwise`
//! - Linux: `$XDG_DATA_HOME/otherwise` (`~/.local/share/otherwise`)
//!
//! `OTHERWISE_DATA_DIR` overrides it (tests, portable setups, a second profile).
//!
//! ```text
//! Otherwise/
//!   hub.json          shared settings (settings.rs)
//!   entities.db       shared entity registry, only once the user opts in (registry.rs)
//!   today/<app>.json  one Today feed per app (today.rs)
//!   today/.dismissed.json
//! ```
use std::path::{Path, PathBuf};

pub const ENV: &str = "OTHERWISE_DATA_DIR";

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn home() -> PathBuf {
    env_dir("HOME").or_else(|| env_dir("USERPROFILE")).unwrap_or_else(|| PathBuf::from("."))
}

/// The shared folder of this user on this machine.
pub fn data_root() -> PathBuf {
    if let Some(d) = env_dir(ENV) {
        return d;
    }
    let home = home();
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("Otherwise")
    } else if cfg!(windows) {
        env_dir("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming")).join("Otherwise")
    } else {
        env_dir("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share")).join("otherwise")
    }
}

/// A shared folder; [`Root::system`] for the real one, [`Root::at`] for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root(pub PathBuf);

impl Root {
    pub fn system() -> Root {
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
}
