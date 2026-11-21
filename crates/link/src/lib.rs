//! The shared integration layer for Otherwise Technologies' desktop apps. No UI, no GPUI, so
//! open-source and closed-source apps can both depend on it (by git tag).
//!
//! - [`link`]: `otherwise://<app>/<path>?…` deep links, parse / build, and [`open::open_link`].
//! - [`register`]: making the OS send an app its links (macOS `CFBundleURLTypes`, Linux
//!   `x-scheme-handler`, Windows `HKCU\Software\Classes`).
//! - [`apps`]: the registry of known apps and detection of the installed ones.
//! - [`entity`] + [`tax`]: versioned entity cards (person, company, vehicle, property, device,
//!   site, digital) with stable UUIDs, aliases and per-app references.
//! - [`registry`]: the opt-in shared SQLite registry of those cards, with a change log and per-app
//!   cursors for incremental sync.
//! - [`today`]: the Today feed (one small JSON file per app, merged by the reader).
//! - [`settings`]: the shared `hub.json` (registry opt-in, MCP write tools).
//! - [`paths`]: where all of this lives (`~/Library/Application Support/Otherwise` and friends).
//!
//! The formats are specified in `docs/SPEC.md`; `docs/INTEGRATING.md` is the how-to for apps.
pub mod apps;
pub mod entity;
pub mod fsutil;
pub mod link;
pub mod open;
pub mod paths;
pub mod register;
pub mod registry;
pub mod settings;
pub mod tax;
pub mod text;
pub mod today;

pub use entity::{Entity, Kind};
pub use link::Link;
pub use paths::Root;
