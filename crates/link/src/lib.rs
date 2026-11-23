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
//! - [`events`] + [`ics`] + [`holidays`]: the calendar feed (`<root>/<app>/events.json`, RRULE
//!   subset, merged and expanded for a range), iCalendar export / import, computed holidays.
//! - [`settings`]: the shared `hub.json` (registry opt-in, MCP write tools).
//! - [`paths`]: where all of this lives: the user's data root (default `~/.otherwise`, movable to
//!   OneDrive & co. through `~/.otherwise/config.json`) and each app's default folder in it.
//! - [`relocate`]: moving the data root (plan, verified copy, pointer) and the one-time move from
//!   the pre-0.2 folder.
//! - [`storage`] + [`cloud`] + [`lock`]: helpers every app uses for its own files: verified
//!   SQLite / file / folder copies and moves, rollback journal, cloud-folder detection, conflicted
//!   copies, and the "open on another computer" lock.
//!
//! The formats are specified in `docs/SPEC.md`; `docs/INTEGRATING.md` is the how-to for apps.
pub mod apps;
pub mod background;
pub mod cloud;
pub mod entity;
pub mod events;
pub mod fsutil;
pub mod holidays;
pub mod ics;
pub mod link;
pub mod lock;
pub mod open;
pub mod paths;
pub mod register;
pub mod registry;
pub mod relocate;
pub mod settings;
pub mod storage;
pub mod tax;
pub mod text;
pub mod today;

pub use entity::{Entity, Kind};
pub use link::Link;
pub use paths::Root;
