//! The client side of an Otherwise server (otherwise-link SPEC §12): an app on a laptop works on
//! the documents of a computer of the tailnet (the always-on Mac Studio) instead of its own.
//!
//! - [`types`]: the `/v1` messages, shared with the servers (the Otherwise Helper's server mode
//!   and, later, Vcity cloud), pinned by the golden vectors in `tests/vectors/`.
//! - [`client`]: blocking HTTP + SSE (`ureq`, rustls). No credentials: identity is Tailscale's.
//! - [`registry`]: `servers.json` in the device folder (servers, each app's active one) and the
//!   replicas' folders.
//! - [`discover`]: the tailnet's computers from the local Tailscale, probed for `/v1/hello`.
//!
//! No UI text: apps word [`Status`] themselves.
pub mod client;
pub mod discover;
pub mod registry;
pub mod sse;
pub mod types;

pub use client::{Client, ClientError, Events, Snapshot};
pub use registry::{Registry, Server, ServerKind};
pub use types::*;

/// How an app shows its connection to the active server (the dot on its tile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Local: the app's own computer.
    Local,
    Connecting,
    /// Connected and allowed to write.
    Connected,
    /// Connected, read-only (`reason`: `role`, `licence`, `newer`, `offline-writes`, or a
    /// server's `read_only_reason`).
    ReadOnly { reason: String },
    /// No answer: the last replica is shown, read-only. `since`: Unix seconds of the last
    /// successful sync.
    Offline { since: Option<i64> },
    /// The app or the server must be updated.
    Incompatible { client_too_old: bool },
    /// `401` / `403`: ask the server's owner for access.
    NoAccess,
}

impl Status {
    /// The status an error leaves the connection in (`since`: last good sync).
    pub fn from_error(e: &ClientError, since: Option<i64>) -> Status {
        match e {
            ClientError::Network(_) | ClientError::Io(_) => Status::Offline { since },
            ClientError::Api { status: 401 | 403 | 404, .. } => Status::NoAccess,
            ClientError::Api { .. } => Status::Offline { since },
            ClientError::Protocol(_) => Status::Incompatible { client_too_old: false },
        }
    }

    pub fn from_doc(h: &DocHello) -> Status {
        if h.writable { Status::Connected } else { Status::ReadOnly { reason: h.read_only_reason.clone().unwrap_or_else(|| "role".into()) } }
    }

    pub fn from_compat(c: Compat) -> Option<Status> {
        match c {
            Compat::Ok => None,
            Compat::ClientTooOld => Some(Status::Incompatible { client_too_old: true }),
            Compat::ServerTooOld | Compat::NotOtherwise => Some(Status::Incompatible { client_too_old: false }),
        }
    }

    pub fn writable(&self) -> bool {
        matches!(self, Status::Local | Status::Connected)
    }
}
