//! The Today feed: what needs the user's attention across apps.
//!
//! Each app owns one file, `today/<app>.json`, and rewrites it whole (atomically, see
//! [`crate::fsutil::write_atomic`]) whenever its list changes: on start, after edits, on its
//! own timer. No app ever writes another app's file, so there is nothing to lock. The reader
//! ([`read`]) merges every file, drops expired events and the ones the user dismissed, and sorts.
//!
//! ```json
//! { "schema": 1, "app": "vigia", "updatedAt": "2026-11-21T08:00:00Z", "events": [
//!   { "id": "item-42-due", "kind": "due", "title": "SOAT ABC123 vence", "when": "2026-11-30",
//!     "severity": "high", "link": "otherwise://vigia/item/42", "expires": "2026-12-07" } ] }
//! ```
//!
//! - `id`: stable per app (the same reminder keeps its id across rewrites, so a dismissal
//!   sticks). Dismissals live in `today/.dismissed.json` (written by Otherwise Apps).
//! - `when`: `YYYY-MM-DD` (a day) or RFC 3339 (a moment). `expires`: same formats; a day means
//!   the end of that day, local time. No `when`: undated (shown under "Anytime").
//! - Limits: [`MAX_EVENTS`] per app, [`MAX_FILE`] bytes per file, titles cut at [`MAX_TITLE`]
//!   characters. Files whose name isn't a valid app id (or start with `.`) are ignored.
use crate::link::valid_app_id;
use crate::paths::Root;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const SCHEMA: u32 = 1;
pub const MAX_EVENTS: usize = 200;
pub const MAX_FILE: u64 = 1 << 20;
pub const MAX_TITLE: usize = 200;
const DISMISSED: &str = ".dismissed.json";
/// Dismissals of events that no longer exist are forgotten after this long.
const DISMISS_KEEP_DAYS: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    /// Something has a deadline (`when`).
    Due,
    /// Something is wrong now (a device down, a certificate expired).
    Alert,
    /// Something waits for the user's review (receipts in the inbox).
    Review,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    /// Filled in by the reader from the file name; writers may leave it out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app: String,
    pub kind: EventKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default)]
    pub severity: Severity,
    /// An `otherwise://` link that opens the thing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    /// The shared entity it is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub schema: u32,
    pub app: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub events: Vec<Event>,
}

/// A parsed `when` / `expires`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Day(NaiveDate),
    At(DateTime<Utc>),
}

impl When {
    pub fn parse(s: &str) -> Option<When> {
        let s = s.trim();
        if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Some(When::Day(d));
        }
        DateTime::parse_from_rfc3339(s).ok().map(|d| When::At(d.with_timezone(&Utc)))
    }

    /// The local calendar day.
    pub fn day<Tz: TimeZone>(&self, tz: &Tz) -> NaiveDate {
        match self {
            When::Day(d) => *d,
            When::At(t) => t.with_timezone(tz).date_naive(),
        }
    }

    /// For sorting: a day sorts at its local midnight.
    fn instant<Tz: TimeZone>(&self, tz: &Tz) -> DateTime<Utc> {
        match self {
            When::Day(d) => tz.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default()).earliest().map(|t| t.with_timezone(&Utc)).unwrap_or_else(|| d.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc()),
            When::At(t) => *t,
        }
    }

    /// For expiry: a day lasts until its local end.
    fn end<Tz: TimeZone>(&self, tz: &Tz) -> DateTime<Utc> {
        match self {
            When::Day(d) => When::Day(d.succ_opt().unwrap_or(*d)).instant(tz),
            When::At(t) => *t,
        }
    }
}

impl Event {
    pub fn new(id: impl Into<String>, kind: EventKind, title: impl Into<String>) -> Event {
        Event { id: id.into(), app: String::new(), kind, title: title.into(), body: None, when: None, severity: Severity::Normal, link: None, expires: None, entity: None }
    }

    pub fn when(mut self, w: impl Into<String>) -> Event {
        self.when = Some(w.into());
        self
    }

    pub fn severity(mut self, s: Severity) -> Event {
        self.severity = s;
        self
    }

    pub fn link(mut self, l: &crate::Link) -> Event {
        self.link = Some(l.to_url());
        self
    }

    pub fn expires(mut self, w: impl Into<String>) -> Event {
        self.expires = Some(w.into());
        self
    }

    pub fn body(mut self, b: impl Into<String>) -> Event {
        self.body = Some(b.into());
        self
    }

    pub fn when_parsed(&self) -> Option<When> {
        self.when.as_deref().and_then(When::parse)
    }

    pub fn expired<Tz: TimeZone>(&self, now: DateTime<Utc>, tz: &Tz) -> bool {
        self.expires.as_deref().and_then(When::parse).is_some_and(|e| e.end(tz) <= now)
    }

    /// `app/id`, the dismissal key.
    pub fn key(&self) -> String {
        format!("{}/{}", self.app, self.id)
    }
}

fn feed_path(root: &Root, app: &str) -> PathBuf {
    root.today_dir().join(format!("{app}.json"))
}

/// Replaces `app`'s feed with `events` (first [`MAX_EVENTS`], titles cut, `app` filled in).
pub fn publish(root: &Root, app: &str, events: &[Event]) -> std::io::Result<()> {
    if !valid_app_id(app) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("invalid app id {app:?}")));
    }
    let mut seen = std::collections::BTreeSet::new();
    let events: Vec<Event> = events
        .iter()
        .filter(|e| !e.id.is_empty() && seen.insert(e.id.clone()))
        .take(MAX_EVENTS)
        .map(|e| {
            let mut e = e.clone();
            e.app = app.to_string();
            if e.title.chars().count() > MAX_TITLE {
                e.title = e.title.chars().take(MAX_TITLE - 1).collect::<String>() + "…";
            }
            e
        })
        .collect();
    let feed = Feed { schema: SCHEMA, app: app.to_string(), updated_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true), events };
    crate::fsutil::write_atomic(&feed_path(root, app), serde_json::to_string_pretty(&feed).unwrap_or_default().as_bytes())
}

/// Removes `app`'s feed (the app was turned off / uninstalled its integration).
pub fn clear(root: &Root, app: &str) -> std::io::Result<()> {
    match std::fs::remove_file(feed_path(root, app)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Merged {
    pub events: Vec<Event>,
    /// app id → when its feed was written.
    pub updated: BTreeMap<String, String>,
    /// Files that couldn't be read: (file name, reason).
    pub errors: Vec<(String, String)>,
}

/// Every feed merged: not expired, not dismissed, sorted by time (undated last), then severity
/// (highest first), then app.
pub fn read(root: &Root, now: DateTime<Utc>) -> Merged {
    read_in(root, now, &Local)
}

pub fn read_in<Tz: TimeZone>(root: &Root, now: DateTime<Utc>, tz: &Tz) -> Merged {
    let mut m = Merged::default();
    let dismissed = Dismissals::load(root);
    let Ok(dir) = std::fs::read_dir(root.today_dir()) else { return m };
    let mut files: Vec<_> = dir.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    for path in files {
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        let Some(app) = name.strip_suffix(".json").filter(|a| valid_app_id(a)).map(str::to_string) else { continue };
        let read = std::fs::metadata(&path).map_err(|e| e.to_string()).and_then(|md| {
            if md.len() > MAX_FILE {
                return Err(format!("larger than {MAX_FILE} bytes"));
            }
            std::fs::read_to_string(&path).map_err(|e| e.to_string())
        });
        let feed = read.and_then(|s| serde_json::from_str::<Feed>(&s).map_err(|e| e.to_string()));
        let feed = match feed {
            Ok(f) if f.schema > SCHEMA => {
                m.errors.push((name, format!("schema {} is newer than {SCHEMA}", f.schema)));
                continue;
            }
            Ok(f) => f,
            Err(e) => {
                m.errors.push((name, e));
                continue;
            }
        };
        m.updated.insert(app.clone(), feed.updated_at.clone());
        for mut e in feed.events.into_iter().take(MAX_EVENTS) {
            // the file name is authoritative: an app can't post as another
            e.app = app.clone();
            if e.expired(now, tz) || dismissed.contains(&e.key()) {
                continue;
            }
            m.events.push(e);
        }
    }
    sort(&mut m.events, tz);
    m
}

pub fn sort<Tz: TimeZone>(events: &mut [Event], tz: &Tz) {
    events.sort_by(|a, b| {
        let (wa, wb) = (a.when_parsed().map(|w| w.instant(tz)), b.when_parsed().map(|w| w.instant(tz)));
        let time = match (wa, wb) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        time.then(b.severity.cmp(&a.severity)).then_with(|| a.app.cmp(&b.app)).then_with(|| a.id.cmp(&b.id))
    });
}

/// Events by local day (None: undated, last), keeping the order within a day. Past days come
/// back as themselves; the UI decides whether to show them as "Overdue".
pub fn by_day<'a, Tz: TimeZone>(events: &'a [Event], tz: &Tz) -> Vec<(Option<NaiveDate>, Vec<&'a Event>)> {
    let mut out: Vec<(Option<NaiveDate>, Vec<&Event>)> = Vec::new();
    for e in events {
        let d = e.when_parsed().map(|w| w.day(tz));
        match out.iter_mut().find(|(k, _)| *k == d) {
            Some((_, v)) => v.push(e),
            None => out.push((d, vec![e])),
        }
    }
    out.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    out
}

/// Events grouped by app, apps in first-appearance order.
pub fn by_app(events: &[Event]) -> Vec<(String, Vec<&Event>)> {
    let mut out: Vec<(String, Vec<&Event>)> = Vec::new();
    for e in events {
        match out.iter_mut().find(|(a, _)| *a == e.app) {
            Some((_, v)) => v.push(e),
            None => out.push((e.app.clone(), vec![e])),
        }
    }
    out
}

/// Events the user dismissed: `app/id` → Unix seconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dismissals(pub BTreeMap<String, i64>);

impl Dismissals {
    pub fn load(root: &Root) -> Dismissals {
        std::fs::read_to_string(root.today_dir().join(DISMISSED)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    pub fn save(&self, root: &Root) -> std::io::Result<()> {
        crate::fsutil::write_atomic(&root.today_dir().join(DISMISSED), serde_json::to_string_pretty(self).unwrap_or_default().as_bytes())
    }
}

/// Hides `e` from the merged feed for good (until its app gives it a new id).
pub fn dismiss(root: &Root, e: &Event, now: DateTime<Utc>) -> std::io::Result<()> {
    let mut d = Dismissals::load(root);
    let cutoff = now.timestamp() - DISMISS_KEEP_DAYS * 86_400;
    d.0.retain(|_, t| *t >= cutoff);
    d.0.insert(e.key(), now.timestamp());
    d.save(root)
}

/// Brings every dismissed event back.
pub fn undismiss_all(root: &Root) -> std::io::Result<()> {
    Dismissals::default().save(root)
}
