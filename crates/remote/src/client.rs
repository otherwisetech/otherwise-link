//! A blocking HTTP client for one server (`https://<host>.<tailnet>.ts.net`). No credentials
//! are sent: the server knows who is asking from Tailscale.
use crate::types::*;
use serde::de::DeserializeOwned;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub enum ClientError {
    /// No answer (offline, DNS, TLS, timeout).
    Network(String),
    /// The server answered with an error.
    Api { status: u16, error: ApiError },
    /// The answer isn't what the contract says.
    Protocol(String),
    Io(std::io::Error),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Network(e) => write!(f, "network: {e}"),
            ClientError::Api { status, error } => write!(f, "{status} {error}"),
            ClientError::Protocol(e) => write!(f, "protocol: {e}"),
            ClientError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl ClientError {
    pub fn code(&self) -> &str {
        match self {
            ClientError::Network(_) => "offline",
            ClientError::Api { error, .. } => &error.code,
            ClientError::Protocol(_) => "protocol",
            ClientError::Io(_) => "io",
        }
    }

    /// The server can't be reached (as opposed to refusing).
    pub fn is_offline(&self) -> bool {
        matches!(self, ClientError::Network(_))
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// Snapshots larger than this are refused (a book's index is a few MB).
pub const MAX_SNAPSHOT: u64 = 2 << 30;
const MAX_JSON: u64 = 16 << 20;

/// What [`Client::snapshot`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot {
    /// The copy at `dest` is current (`304`).
    NotModified,
    Saved { revision: u64, etag: String, bytes: u64 },
}

/// Document and app ids in paths: `[a-z0-9-_.]`, no leading dot, at most 80.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 80 && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'))
}

#[derive(Clone)]
pub struct Client {
    base: String,
    agent: ureq::Agent,
    stream_agent: ureq::Agent,
}

impl Client {
    /// `base`: scheme and host (`https://studio.tail4cc17.ts.net`), see
    /// [`crate::registry::normalize_url`].
    pub fn new(base: &str) -> Client {
        Client::with_timeout(base, Duration::from_secs(30))
    }

    pub fn with_timeout(base: &str, timeout: Duration) -> Client {
        let mk = |global: Option<Duration>| -> ureq::Agent {
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_connect(Some(timeout.min(Duration::from_secs(10))))
                .timeout_global(global)
                // a server answers itself; a redirect elsewhere is refused
                .max_redirects(0)
                .user_agent(concat!("otherwise-remote/", env!("CARGO_PKG_VERSION")))
                .build()
                .into()
        };
        // a stream is closed by the server every few minutes ("rotate"); the cap bounds a dead
        // connection the OS hasn't noticed yet
        Client { base: base.trim_end_matches('/').to_string(), agent: mk(Some(timeout)), stream_agent: mk(Some(Duration::from_secs(6 * 60))) }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{API}{path}", self.base)
    }

    fn doc_path(app: &str, doc: &str, what: &str) -> Result<String> {
        if !valid_id(app) || !valid_id(doc) {
            return Err(ClientError::Protocol(format!("bad id {app}/{doc}")));
        }
        Ok(format!("/apps/{app}/docs/{doc}{what}"))
    }

    fn read<T: DeserializeOwned>(resp: ureq::http::Response<ureq::Body>) -> Result<T> {
        let status = resp.status().as_u16();
        let text = resp.into_body().into_with_config().limit(MAX_JSON).read_to_string().map_err(|e| ClientError::Network(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(api_error(status, &text));
        }
        serde_json::from_str(&text).map_err(|e| ClientError::Protocol(format!("{e}: {}", truncate(&text, 200))))
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.agent.get(&self.url(path)).header("Accept", "application/json").call().map_err(net)?;
        Self::read(resp)
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &impl serde::Serialize) -> Result<T> {
        let resp = self.agent.post(&self.url(path)).header("Accept", "application/json").send_json(body).map_err(net)?;
        Self::read(resp)
    }

    pub fn hello(&self) -> Result<Hello> {
        let h: Hello = self.get("/hello")?;
        if h.server != SERVER {
            return Err(ClientError::Protocol(format!("not an Otherwise server: {}", h.server)));
        }
        Ok(h)
    }

    pub fn docs(&self, app: &str) -> Result<Vec<DocInfo>> {
        if !valid_id(app) {
            return Err(ClientError::Protocol(format!("bad app {app}")));
        }
        Ok(self.get::<DocList>(&format!("/apps/{app}/docs"))?.docs)
    }

    pub fn doc_hello(&self, app: &str, doc: &str) -> Result<DocHello> {
        self.get(&Self::doc_path(app, doc, "/hello")?)
    }

    /// Download the document's snapshot to `dest` (atomically, `0600`) unless `etag` is still
    /// current.
    pub fn snapshot(&self, app: &str, doc: &str, etag: Option<&str>, dest: &Path) -> Result<Snapshot> {
        let mut req = self.agent.get(&self.url(&Self::doc_path(app, doc, "/snapshot")?));
        if let Some(e) = etag {
            req = req.header("If-None-Match", &format!("\"{e}\""));
        }
        let resp = req.call().map_err(net)?;
        let status = resp.status().as_u16();
        if status == 304 {
            return Ok(Snapshot::NotModified);
        }
        if status != 200 {
            let text = resp.into_body().into_with_config().limit(MAX_JSON).read_to_string().unwrap_or_default();
            return Err(api_error(status, &text));
        }
        let tag = resp.headers().get("ETag").and_then(|v| v.to_str().ok()).map(|s| s.trim_start_matches("W/").trim_matches('"').to_string()).unwrap_or_default();
        let revision = resp.headers().get("X-Otherwise-Revision").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok()).or_else(|| tag.parse().ok()).unwrap_or(0);
        let mut body = resp.into_body().into_with_config().limit(MAX_SNAPSHOT).reader();
        let bytes = write_private(dest, &mut body)?;
        Ok(Snapshot::Saved { revision, etag: tag, bytes })
    }

    pub fn op(&self, app: &str, doc: &str, req: &OpRequest) -> Result<OpResult> {
        self.post(&Self::doc_path(app, doc, "/ops")?, req)
    }

    pub fn undo(&self, app: &str, doc: &str, req: &UndoRequest) -> Result<OpResult> {
        self.post(&Self::doc_path(app, doc, "/undo")?, req)
    }

    /// Open the live events; reconnect with the stream's [`Events::last_id`] after it ends.
    pub fn events(&self, app: &str, doc: &str, last_event_id: Option<&str>) -> Result<Events> {
        let mut req = self.stream_agent.get(&self.url(&Self::doc_path(app, doc, "/events")?)).header("Accept", "text/event-stream");
        if let Some(id) = last_event_id {
            req = req.header("Last-Event-ID", id);
        }
        let resp = req.call().map_err(net)?;
        let status = resp.status().as_u16();
        if status != 200 {
            let text = resp.into_body().into_with_config().limit(MAX_JSON).read_to_string().unwrap_or_default();
            return Err(api_error(status, &text));
        }
        let reader: Box<dyn Read + Send> = Box::new(resp.into_body().into_with_config().limit(u64::MAX).reader());
        Ok(Events { inner: crate::sse::Reader::new(BufReader::new(reader)) })
    }
}

/// A live `…/events` stream. Iterating ends when the server closes it (or the connection
/// drops): reconnect.
pub struct Events {
    inner: crate::sse::Reader<BufReader<Box<dyn Read + Send>>>,
}

impl Events {
    pub fn last_id(&self) -> Option<&str> {
        self.inner.last_id()
    }
}

impl Iterator for Events {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        loop {
            let m = self.inner.next_message().ok()??;
            match serde_json::from_str::<Event>(&m.data) {
                Ok(Event::Unknown) | Err(_) => continue,
                Ok(e) => return Some(e),
            }
        }
    }
}

fn net(e: ureq::Error) -> ClientError {
    ClientError::Network(e.to_string())
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn api_error(status: u16, text: &str) -> ClientError {
    let error = serde_json::from_str::<serde_json::Value>(text).ok().and_then(|v| v.get("error").cloned()).and_then(|e| serde_json::from_value::<ApiError>(e).ok()).unwrap_or_else(|| {
        let code = match status {
            401 => "unauthorized",
            403 => "forbidden",
            404 => "not_found",
            503 => "unavailable",
            _ => "failed",
        };
        ApiError::new(code, &truncate(text, 200))
    });
    ClientError::Api { status, error }
}

/// Copy `from` into `dest` through a hidden temp file next to it, `0600`, then rename.
pub(crate) fn write_private(dest: &Path, from: &mut dyn Read) -> Result<u64> {
    let dir = dest.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(ClientError::Io)?;
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.part", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let res = (|| {
        let mut f = opts.open(&tmp)?;
        let n = std::io::copy(from, &mut f)?;
        f.flush()?;
        f.sync_all()?;
        std::fs::rename(&tmp, dest)?;
        Ok::<u64, std::io::Error>(n)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.map_err(|e| if e.kind() == std::io::ErrorKind::Other { ClientError::Network(e.to_string()) } else { ClientError::Io(e) })
}
