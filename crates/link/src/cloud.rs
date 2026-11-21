//! Cloud-synced folders: is a path inside iCloud Drive, OneDrive, Dropbox, Google Drive or Box,
//! and has the sync client left "conflicted copies" of a file next to it?
//!
//! Detection is by path only (no provider APIs): macOS File Provider folders
//! (`~/Library/CloudStorage/<Provider>-…`), iCloud Drive (`~/Library/Mobile Documents`), the
//! classic home folders (`~/Dropbox`, `~/OneDrive…`, `~/Google Drive`, `~/iCloudDrive`, `~/Box`),
//! Google Drive's `My Drive` / `Shared drives` anywhere (Windows mounts it as a drive letter) and
//! Windows' `%OneDrive%` / `%OneDriveConsumer%` / `%OneDriveCommercial%`.
//!
//! What to do in one: SQLite with the rollback journal (never WAL: [`crate::storage::
//! ensure_rollback_journal`]), a lock file ([`crate::lock`]) so two computers don't write the same
//! file at once, and a warning when [`conflicted_copies`] finds any.
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    ICloud,
    OneDrive,
    Dropbox,
    GoogleDrive,
    Box,
    /// Another File Provider (`~/Library/CloudStorage/<name>`).
    Other,
}

impl Provider {
    /// Stable id (`icloud`, `onedrive`, `dropbox`, `googledrive`, `box`, `other`) for apps to key
    /// their translations on.
    pub fn id(self) -> &'static str {
        match self {
            Provider::ICloud => "icloud",
            Provider::OneDrive => "onedrive",
            Provider::Dropbox => "dropbox",
            Provider::GoogleDrive => "googledrive",
            Provider::Box => "box",
            Provider::Other => "other",
        }
    }

    /// The product name (a brand: not translated).
    pub fn name(self) -> &'static str {
        match self {
            Provider::ICloud => "iCloud Drive",
            Provider::OneDrive => "OneDrive",
            Provider::Dropbox => "Dropbox",
            Provider::GoogleDrive => "Google Drive",
            Provider::Box => "Box",
            Provider::Other => "Cloud",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudFolder {
    pub provider: Provider,
    /// The synced folder's top (e.g. `~/Library/CloudStorage/OneDrive-Personal`).
    pub root: PathBuf,
}

fn by_folder_name(name: &str) -> Option<Provider> {
    let n = name.to_ascii_lowercase();
    if n == "dropbox" || n.starts_with("dropbox (") || n.starts_with("dropbox-") {
        Some(Provider::Dropbox)
    } else if n == "onedrive" || n.starts_with("onedrive -") || n.starts_with("onedrive-") {
        Some(Provider::OneDrive)
    } else if n == "google drive" || n.starts_with("googledrive") || n == "my drive" {
        Some(Provider::GoogleDrive)
    } else if n == "icloud drive" || n == "iclouddrive" {
        Some(Provider::ICloud)
    } else if n == "box" || n == "box sync" || n.starts_with("box-") {
        Some(Provider::Box)
    } else {
        None
    }
}

fn names(p: &Path) -> Vec<String> {
    p.components().filter_map(|c| if let Component::Normal(s) = c { Some(s.to_string_lossy().into_owned()) } else { None }).collect()
}

/// The cloud folder `path` is in, judged from the path alone. `home` is the user's home,
/// `onedrive` the extra OneDrive roots (Windows' environment variables).
pub fn detect_in(path: &Path, home: &Path, onedrive: &[PathBuf]) -> Option<CloudFolder> {
    for r in onedrive.iter().filter(|r| !r.as_os_str().is_empty()) {
        if path.starts_with(r) {
            return Some(CloudFolder { provider: Provider::OneDrive, root: r.clone() });
        }
    }
    if let Ok(rest) = path.strip_prefix(home) {
        let n = names(rest);
        let eq = |i: usize, s: &str| n.get(i).is_some_and(|x| x.eq_ignore_ascii_case(s));
        if eq(0, "Library") && eq(1, "CloudStorage") && n.len() > 2 {
            let provider = by_folder_name(&n[2]).unwrap_or(Provider::Other);
            return Some(CloudFolder { provider, root: home.join("Library").join("CloudStorage").join(&n[2]) });
        }
        if eq(0, "Library") && eq(1, "Mobile Documents") {
            let mut root = home.join("Library").join("Mobile Documents");
            if let Some(c) = n.get(2) {
                root.push(c);
            }
            return Some(CloudFolder { provider: Provider::ICloud, root });
        }
        if let Some(provider) = n.first().and_then(|f| by_folder_name(f)) {
            return Some(CloudFolder { provider, root: home.join(&n[0]) });
        }
    }
    // Google Drive for desktop on Windows: `G:\My Drive\…`, `G:\Shared drives\…`
    let mut acc = PathBuf::new();
    for c in path.components() {
        acc.push(c);
        if let Component::Normal(s) = c {
            let s = s.to_string_lossy();
            if s.eq_ignore_ascii_case("My Drive") || s.eq_ignore_ascii_case("Shared drives") {
                return Some(CloudFolder { provider: Provider::GoogleDrive, root: acc });
            }
        }
    }
    None
}

/// The deepest existing ancestor, canonicalized (so a `~/OneDrive` symlink into
/// `~/Library/CloudStorage` counts), plus the part that doesn't exist yet.
fn canonical(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for m in missing.iter().rev() {
                out.push(m);
            }
            return out;
        }
        match (cur.file_name().map(|f| f.to_os_string()), cur.parent()) {
            (Some(f), Some(p)) => {
                missing.push(f);
                cur = p.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// [`detect_in`] for this user, after resolving symlinks.
pub fn detect(path: &Path) -> Option<CloudFolder> {
    let home = crate::paths::home();
    let onedrive: Vec<PathBuf> = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"].iter().filter_map(std::env::var_os).filter(|v| !v.is_empty()).map(PathBuf::from).collect();
    detect_in(path, &home, &onedrive).or_else(|| {
        let c = canonical(path);
        let ch = canonical(&home);
        let co: Vec<PathBuf> = onedrive.iter().map(|o| canonical(o)).collect();
        detect_in(&c, &ch, &co)
    })
}

/// Does `candidate` look like a sync client's conflict copy of `file`? Dropbox / Nextcloud
/// `name (… conflicted copy …).ext`, Syncthing `name.sync-conflict-….ext`, iCloud `name 2.ext`,
/// Google Drive / OneDrive `name (1).ext`, and OneDrive's `name-<computer>.ext` when
/// `provider` is OneDrive.
pub fn is_conflict_name(file: &str, candidate: &str, provider: Option<Provider>) -> bool {
    if file == candidate {
        return false;
    }
    let split = |s: &str| -> (String, String) {
        match s.rfind('.') {
            Some(i) if i > 0 => (s[..i].to_string(), s[i..].to_ascii_lowercase()),
            _ => (s.to_string(), String::new()),
        }
    };
    let (stem, ext) = split(file);
    let (cstem, cext) = split(candidate);
    if ext != cext || !cstem.starts_with(&stem) {
        return false;
    }
    let rest = &cstem[stem.len()..];
    if rest.is_empty() {
        return false;
    }
    let lower = rest.to_ascii_lowercase();
    if lower.contains("conflict") {
        return true;
    }
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if let Some(n) = rest.strip_prefix(' ') {
        if digits(n) {
            return true;
        }
        if let Some(inner) = n.strip_prefix('(').and_then(|x| x.strip_suffix(')'))
            && digits(inner)
        {
            return true;
        }
    }
    if provider == Some(Provider::OneDrive)
        && let Some(h) = rest.strip_prefix('-')
    {
        return !h.is_empty() && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    }
    false
}

/// Conflict copies of `file` in its folder (sorted). The provider comes from [`detect`].
pub fn conflicted_copies(file: &Path) -> Vec<PathBuf> {
    conflicted_copies_with(file, detect(file).map(|c| c.provider))
}

pub fn conflicted_copies_with(file: &Path, provider: Option<Provider>) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name().map(|n| n.to_string_lossy().into_owned())) else { return Vec::new() };
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| is_conflict_name(&name, &e.file_name().to_string_lossy(), provider))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}
