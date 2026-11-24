//! `otherwise-hub`: one MCP server (stdio, JSON-RPC 2.0, one message per line) for every
//! Otherwise app installed here.
//!
//! - **Proxied tools.** Apps that ship `<cli> mcp` (see `otherwise_link::apps`) are started as
//!   child MCP servers; their tools are offered as `<app>.<tool>` (`expense.search_transactions`,
//!   `vigia.due`, `nodo.devices`). A child that fails to start or answer is skipped; one that
//!   dies is restarted once on the next call.
//! - **Own tools.** `entities_search`, `entities_get`, `today`, `open_link`, and with writes on
//!   `entities_upsert` and `entities_link` (see [`tools`]).
//! - **Read-only by default.** Write tools (ours, and every child tool not annotated
//!   `readOnlyHint: true`) are offered only when the user turned on "Allow write tools" in
//!   Otherwise Apps (`hub.json` → `mcpWrites`), and children are started with their write flag
//!   (`--allow-writes`) only then. The setting is checked again on every write call, so turning
//!   it off takes effect at once.
//! - **Read-only fallback.** An app that refuses its write flag at start (it exits, closes or
//!   refuses `initialize`: expense does when the book doesn't allow agent changes) is started
//!   again without it and offered read-only for the rest of the hub session; the hub logs it once
//!   on stderr and `ChildStatus::read_only` says why. The fallback only removes write permission.
pub mod child;
pub mod hub;
pub mod rpc;
pub mod tools;

pub use hub::{Config, Hub};

/// Serves stdin / stdout until stdin closes, with the system configuration. What the
/// `otherwise-hub` binary (and `otherwise-apps mcp`) run.
pub fn run_stdio() -> std::io::Result<()> {
    let mut hub = Hub::start(Config::system());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    hub.serve(stdin.lock(), stdout.lock())
}
