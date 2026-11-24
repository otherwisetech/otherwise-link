//! The `/v1` messages (SPEC §12). Every struct ignores fields it doesn't know and every new
//! field is optional, so older and newer clients and servers keep talking.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `hello.server` of an Otherwise server.
pub const SERVER: &str = "otherwise";
/// The HTTP contract version this crate speaks (`/v1`).
pub const API: &str = "v1";
/// Range of `protocols` this crate understands.
pub const PROTOCOL_MIN: u32 = 1;
pub const PROTOCOL_MAX: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protocols {
    pub min: u32,
    pub max: u32,
}

/// Who the server says is asking (from Tailscale: login, display name, device).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub login: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub device: String,
    /// The member's server-wide role, when the server knows them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub protocol: u32,
}

/// `GET /v1/hello`: no business data, so "Add a server" can call it before anything is shared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub server: String,
    #[serde(default)]
    pub version: String,
    pub protocols: Protocols,
    /// The server computer's name ("studio").
    #[serde(default)]
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub me: Option<Me>,
    #[serde(default)]
    pub apps: Vec<AppInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    Ok,
    /// The server only speaks protocols newer than ours: update the app.
    ClientTooOld,
    /// The server only speaks protocols older than ours.
    ServerTooOld,
    /// Not an Otherwise server.
    NotOtherwise,
}

impl Hello {
    pub fn compat(&self) -> Compat {
        if self.server != SERVER {
            Compat::NotOtherwise
        } else if self.protocols.min > PROTOCOL_MAX {
            Compat::ClientTooOld
        } else if self.protocols.max < PROTOCOL_MIN {
            Compat::ServerTooOld
        } else {
            Compat::Ok
        }
    }

    pub fn app(&self, id: &str) -> Option<&AppInfo> {
        self.apps.iter().find(|a| a.id == id)
    }
}

/// A member's role on a document (SPEC §12.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Reads only.
    Viewer,
    /// An AI agent: writes only what the document lets agents write; never people-only
    /// operations (approving, closing a month).
    Agent,
    /// Writes, undoes their own changes, accepts proposals.
    Editor,
    /// Everything, including members, sharing, the audit log and undoing anyone's change.
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Agent => "agent",
            Role::Editor => "editor",
            Role::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        Some(match s {
            "viewer" => Role::Viewer,
            "agent" => Role::Agent,
            "editor" => Role::Editor,
            "owner" => Role::Owner,
            _ => return None,
        })
    }

    pub fn can_write(self) -> bool {
        self != Role::Viewer
    }

    pub fn undoes_anyones(self) -> bool {
        self == Role::Owner
    }
}

/// One entry of `GET /v1/apps/<app>/docs`: a document shared with me.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocInfo {
    pub id: String,
    pub name: String,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocList {
    pub docs: Vec<DocInfo>,
}

/// Seats in the team's licence tier, counted by the server (SPEC §12.6). Shown to owners only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seats {
    /// Distinct identities with an authenticated document request in the last 30 days.
    pub active: u32,
    /// The tier's maximum, when a tier is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    /// First day (`YYYY-MM-DD`) above the tier, while still above it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over_since: Option<String>,
}

/// `GET /v1/apps/<app>/docs/<doc>/hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocHello {
    pub app: String,
    pub doc: String,
    #[serde(default)]
    pub name: String,
    pub role: Role,
    /// Role and document both allow writing now.
    pub writable: bool,
    pub revision: u64,
    #[serde(default)]
    pub protocol: u32,
    /// Why not writable (`role`, `read_only`, `licence`, `newer`…), when it isn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seats: Option<Seats>,
}

/// `POST …/ops`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpRequest {
    pub op: String,
    #[serde(default)]
    pub args: Value,
    /// Refuse with `conflict` unless the document is still at this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    /// A fresh random id per user action; a retry with the same key gets the first answer and
    /// writes nothing again.
    pub idempotency_key: String,
    /// What the user did, for the document's change journal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// The answer to `ops` / `undo`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpResult {
    pub ok: bool,
    #[serde(default)]
    pub result: Value,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
    /// This answer was stored for the idempotency key (a retry).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replayed: bool,
}

/// `POST …/undo`. No `change_id`: my last change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
    pub idempotency_key: String,
}

/// `{"ok": false, "error": {"code", "message", …}}`. Extra fields (`rows`, `month`, `field`…)
/// travel in `extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ApiError {
    pub fn new(code: &str, message: &str) -> ApiError {
        ApiError { code: code.into(), message: message.into(), extra: Map::new() }
    }

    pub fn with(mut self, k: &str, v: Value) -> ApiError {
        self.extra.insert(k.into(), v);
        self
    }

    pub fn envelope(&self) -> Value {
        serde_json::json!({"ok": false, "error": self})
    }

    /// The HTTP status a server answers this code with (SPEC §12.5).
    pub fn status(&self) -> u16 {
        status_for(&self.code)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() { write!(f, "{}", self.code) } else { write!(f, "{}: {}", self.code, self.message) }
    }
}

/// Error codes and their HTTP status. Operation errors of an app (`not_found`, `invalid`,
/// `locked`…) pass through with the status of their family.
pub fn status_for(code: &str) -> u16 {
    match code {
        "bad_request" | "invalid" | "unknown_op" => 400,
        "unauthorized" => 401,
        "licence" => 402,
        "forbidden" | "people_only" => 403,
        "not_found" => 404,
        "conflict" | "read_only" | "locked" | "exists" | "already_undone" | "nothing_to_undo" | "busy" | "not_ready" => 409,
        "too_large" => 413,
        "unavailable" => 503,
        _ => 500,
    }
}

/// A live event on `GET …/events` (SSE: `event:` is the tag, `id:` the revision, `data:` this
/// JSON). Unknown kinds parse as [`Event::Unknown`] and are skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The document moved to `revision` (any writer: a client, the server's own jobs, the app
    /// open on the server). Fetch a new snapshot.
    Changed {
        revision: u64,
        #[serde(default)]
        tables: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor: Option<String>,
    },
    /// A background job of the app on the server ran.
    Job { id: String, status: String },
    /// The server is closing this stream (`update`, `shutdown`, `idle`, `rotate`): reconnect
    /// with `Last-Event-ID`.
    Bye { reason: String },
    #[serde(other)]
    Unknown,
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::Changed { .. } => "changed",
            Event::Job { .. } => "job",
            Event::Bye { .. } => "bye",
            Event::Unknown => "unknown",
        }
    }
}

/// A random idempotency key: 128 bits, 32 hex digits (`/dev/urandom`; elsewhere std's
/// randomly keyed hasher, which the OS seeds).
pub fn new_key() -> String {
    let mut b = [0u8; 16];
    if fill_random(&mut b).is_err() {
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        for half in 0..2 {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(t);
            h.write_u64(N.fetch_add(1, Ordering::Relaxed));
            h.write_u32(std::process::id());
            h.write_u8(half);
            b[half as usize * 8..half as usize * 8 + 8].copy_from_slice(&h.finish().to_le_bytes());
        }
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn fill_random(b: &mut [u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")?.read_exact(b)
    }
    #[cfg(not(unix))]
    {
        let _ = b;
        Err(std::io::Error::other("no /dev/urandom"))
    }
}
