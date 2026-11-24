//! `servers.json` in this computer's device folder (otherwise-link `paths::device_dir()`, passed
//! in): the servers the user added, shared by every app, and each app's active server and
//! document. "Local" is not stored as a server: it is the absence of an active one.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "servers.json";
pub const SCHEMA: u32 = 1;
/// The id the apps use for their own computer.
pub const LOCAL: &str = "local";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerKind {
    /// A computer of the user's tailnet (identity from Tailscale).
    #[default]
    Tailnet,
    /// A server of a customer's own network.
    OnPrem,
    /// Vcity cloud (signed in with the Vcity session). Not served yet.
    Vcity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: String,
    pub name: String,
    /// Scheme and host, no path (`https://studio.tail4cc17.ts.net`).
    pub url: String,
    #[serde(default)]
    pub kind: ServerKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Active {
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub schema: u32,
    #[serde(default)]
    pub servers: Vec<Server>,
    /// Per app id.
    #[serde(default)]
    pub active: BTreeMap<String, Active>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry { schema: SCHEMA, servers: Vec::new(), active: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    /// Only `https://` (and `http://` for a loopback test server) are accepted.
    Scheme,
    Invalid,
}

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            UrlError::Empty => "empty address",
            UrlError::Scheme => "only https:// addresses",
            UrlError::Invalid => "not a host name",
        })
    }
}

/// What the user typed ("studio.tail4cc17.ts.net", "https://studio.tail4cc17.ts.net/") as the
/// base URL: `https://` added, path, query and trailing dot dropped, host lowercased.
pub fn normalize_url(input: &str) -> Result<String, UrlError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(UrlError::Empty);
    }
    let (scheme, rest) = match s.split_once("://") {
        Some((sc, r)) => (sc.to_ascii_lowercase(), r),
        None => ("https".to_string(), s),
    };
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    let host = host_port.trim_end_matches('.').to_string();
    let bare = host.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map(|(h, _)| h).unwrap_or(&host);
    let bare = bare.trim_end_matches('.');
    if bare.is_empty() || !bare.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '[' | ']' | ':')) || bare.contains('@') {
        return Err(UrlError::Invalid);
    }
    let loopback = bare == "localhost" || bare == "127.0.0.1" || bare == "[::1]";
    match scheme.as_str() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err(UrlError::Scheme),
    }
    Ok(format!("{scheme}://{host}"))
}

/// The server's first DNS label, as a name ("studio" → "Studio"); the UI may rename it.
pub fn default_name(url: &str) -> String {
    let host = url.split("://").nth(1).unwrap_or(url);
    let first = host.split(['.', ':']).next().unwrap_or(host);
    let mut c = first.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').chars().take(40).collect::<String>();
    if out.is_empty() { "server".into() } else { out }
}

impl Registry {
    pub fn path(device_dir: &Path) -> PathBuf {
        device_dir.join(FILE)
    }

    /// The file, or an empty list when it is missing, unreadable or of a newer schema (a newer
    /// app's file is never rewritten: [`Registry::save`] refuses).
    pub fn load(device_dir: &Path) -> Registry {
        std::fs::read_to_string(Self::path(device_dir)).ok().and_then(|s| serde_json::from_str::<Registry>(&s).ok()).filter(|r| r.schema <= SCHEMA).unwrap_or_default()
    }

    pub fn save(&self, device_dir: &Path) -> std::io::Result<()> {
        if let Ok(s) = std::fs::read_to_string(Self::path(device_dir))
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&s)
            && v.get("schema").and_then(|s| s.as_u64()).is_some_and(|s| s > SCHEMA as u64)
        {
            return Err(std::io::Error::other("servers.json is from a newer app"));
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::client::write_private(&Self::path(device_dir), &mut bytes.as_slice()).map(|_| ()).map_err(|e| std::io::Error::other(e.to_string()))
    }

    pub fn get(&self, id: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// Add a server (or return the one with the same URL). The id is stable for the replicas'
    /// folder.
    pub fn add(&mut self, name: &str, url: &str, kind: ServerKind) -> Result<String, UrlError> {
        let url = normalize_url(url)?;
        if let Some(s) = self.servers.iter().find(|s| s.url == url) {
            return Ok(s.id.clone());
        }
        let name = if name.trim().is_empty() { default_name(&url) } else { name.trim().to_string() };
        let base = slug(url.split("://").nth(1).unwrap_or(&url).split('.').next().unwrap_or("server"));
        let mut id = base.clone();
        let mut n = 2;
        while id == LOCAL || self.get(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        self.servers.push(Server { id: id.clone(), name, url, kind });
        Ok(id)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> bool {
        match self.servers.iter_mut().find(|s| s.id == id) {
            Some(s) if !name.trim().is_empty() => {
                s.name = name.trim().to_string();
                true
            }
            _ => false,
        }
    }

    /// Forget a server; apps that used it go back to Local. The caller deletes the server's
    /// replicas ([`server_dir`]).
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.servers.len();
        self.servers.retain(|s| s.id != id);
        self.active.retain(|_, a| a.server != id);
        before != self.servers.len()
    }

    /// The app's active server (`None`: Local).
    pub fn active(&self, app: &str) -> Option<(&Server, Option<&str>)> {
        let a = self.active.get(app)?;
        Some((self.get(&a.server)?, a.doc.as_deref()))
    }

    /// `server`: a server id, or [`LOCAL`].
    pub fn set_active(&mut self, app: &str, server: &str, doc: Option<&str>) {
        if server == LOCAL || self.get(server).is_none() {
            self.active.remove(app);
        } else {
            self.active.insert(app.into(), Active { server: server.into(), doc: doc.map(str::to_string) });
        }
    }
}

/// Every replica of a server: `<device>/remote/<server>/`.
pub fn server_dir(device_dir: &Path, server: &str) -> PathBuf {
    device_dir.join("remote").join(slug(server))
}

/// The folder an app keeps one document's read-only replica in:
/// `<device>/remote/<server>/<app>/<doc>/`.
pub fn replica_dir(device_dir: &Path, server: &str, app: &str, doc: &str) -> PathBuf {
    server_dir(device_dir, server).join(slug(app)).join(slug(doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalized_and_only_https_leaves_the_computer() {
        assert_eq!(normalize_url("studio.tail4cc17.ts.net").unwrap(), "https://studio.tail4cc17.ts.net");
        assert_eq!(normalize_url(" HTTPS://Studio.tail4cc17.ts.net./v1/hello?x ").unwrap(), "https://studio.tail4cc17.ts.net");
        assert_eq!(normalize_url("http://127.0.0.1:47900").unwrap(), "http://127.0.0.1:47900");
        assert_eq!(normalize_url("http://studio.tail4cc17.ts.net"), Err(UrlError::Scheme));
        assert_eq!(normalize_url("ftp://x"), Err(UrlError::Scheme));
        assert_eq!(normalize_url("user@host"), Err(UrlError::Invalid));
        assert_eq!(normalize_url("  "), Err(UrlError::Empty));
        assert_eq!(default_name("https://studio.tail4cc17.ts.net"), "Studio");
    }

    #[test]
    fn add_remove_and_active_per_app() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::load(d.path());
        assert!(r.servers.is_empty());
        let id = r.add("Mac Studio", "studio.tail4cc17.ts.net", ServerKind::Tailnet).unwrap();
        assert_eq!(id, "studio");
        assert_eq!(r.add("", "https://studio.tail4cc17.ts.net/", ServerKind::Tailnet).unwrap(), "studio", "same URL, same server");
        let other = r.add("", "https://studio.other.ts.net", ServerKind::Tailnet).unwrap();
        assert_eq!(other, "studio-2");
        r.set_active("expense", &id, Some("acme"));
        r.set_active("brujula", LOCAL, None);
        r.save(d.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(Registry::path(d.path())).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut r = Registry::load(d.path());
        let (s, doc) = r.active("expense").unwrap();
        assert_eq!((s.name.as_str(), doc), ("Mac Studio", Some("acme")));
        assert!(r.active("brujula").is_none(), "Local is no entry");
        assert!(r.remove(&id));
        assert!(r.active("expense").is_none(), "its apps fall back to Local");
        assert_eq!(replica_dir(d.path(), "studio", "expense", "acme"), d.path().join("remote/studio/expense/acme"));
        assert_eq!(replica_dir(d.path(), "../x", "expense", "a/b"), d.path().join("remote/x/expense/a-b"), "no path escapes");
    }

    #[test]
    fn a_newer_file_is_read_as_empty_and_never_overwritten() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(Registry::path(d.path()), r#"{"schema": 9, "servers": []}"#).unwrap();
        let r = Registry::load(d.path());
        assert!(r.save(d.path()).is_err());
        assert!(std::fs::read_to_string(Registry::path(d.path())).unwrap().contains("\"schema\": 9"));
    }
}
