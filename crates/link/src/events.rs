//! The events feed: what each app has on the calendar (due dates, renewals, payments,
//! birthdays, audits…), merged by Otherwise Hub's Calendar view.
//!
//! Each app owns one file, `<data root>/<app>/events.json`, and rewrites it whole (atomically)
//! when its events change. The reader ([`read`]) merges every app's file and expands
//! recurrences ([`Rule`], an RRULE subset) into [`Occurrence`]s for a date range.
//!
//! ```json
//! { "schema": 1, "app": "vigia", "updatedAt": "2026-11-21T08:00:00Z", "events": [
//!   { "id": "item-42", "title": "SOAT ABC123 vence", "start": "2026-11-30", "kind": "due",
//!     "link": "otherwise://vigia/item/42", "entities": ["6f0c…"] },
//!   { "id": "pay-rent", "title": "Arriendo", "start": "2026-01-05", "rrule": "FREQ=MONTHLY",
//!     "kind": "payment", "busyOnly": true } ] }
//! ```
//!
//! - `start`: `YYYY-MM-DD` (all day) or RFC 3339 (a moment, with its offset). `end`: for an
//!   all-day event the **last day, inclusive**; for a timed one the end instant (exclusive).
//! - `rrule`: `FREQ=DAILY|WEEKLY|MONTHLY|YEARLY` with `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY`
//!   (`MO,WE` / `2MO` / `-1FR`), `BYMONTH`, `BYMONTHDAY` (`-1` = last day). Days a rule
//!   produces that don't exist (Feb 30) are skipped, as in RFC 5545. `exdates` removes days.
//!   Instead of a rule an app may write expanded instances (separate events sharing `series`).
//! - `busyOnly`: private details. Readers show the title only in the user's own calendar UI;
//!   exports (ICS) and assistants get [`Occurrence::redacted`] (no title, body, location).
use crate::link::valid_app_id;
use crate::paths::Root;
use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const SCHEMA: u32 = 1;
pub const FILE: &str = "events.json";
pub const MAX_EVENTS: usize = 5000;
pub const MAX_FILE: u64 = 4 << 20;
pub const MAX_TITLE: usize = 200;
/// Occurrences one event may produce in one read (a daily rule over a long range).
pub const MAX_OCCURRENCES: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A deadline (a document expires, a filing is due).
    Due,
    Renewal,
    /// A heads-up day before something that has its own event (Vigía's "30 days before");
    /// the Hub hides these unless the user asks for them.
    Reminder,
    Payment,
    Payroll,
    /// A tax or legal deadline.
    Tax,
    /// A period close (month close).
    Close,
    Birthday,
    Anniversary,
    Meeting,
    Booking,
    Task,
    Audit,
    Maintenance,
    Warranty,
    Holiday,
    #[default]
    #[serde(other)]
    Other,
}

impl Kind {
    pub const ALL: [Kind; 17] = [Kind::Due, Kind::Renewal, Kind::Reminder, Kind::Payment, Kind::Payroll, Kind::Tax, Kind::Close, Kind::Birthday, Kind::Anniversary, Kind::Meeting, Kind::Booking, Kind::Task, Kind::Audit, Kind::Maintenance, Kind::Warranty, Kind::Holiday, Kind::Other];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Due => "due",
            Kind::Renewal => "renewal",
            Kind::Reminder => "reminder",
            Kind::Payment => "payment",
            Kind::Payroll => "payroll",
            Kind::Tax => "tax",
            Kind::Close => "close",
            Kind::Birthday => "birthday",
            Kind::Anniversary => "anniversary",
            Kind::Meeting => "meeting",
            Kind::Booking => "booking",
            Kind::Task => "task",
            Kind::Audit => "audit",
            Kind::Maintenance => "maintenance",
            Kind::Warranty => "warranty",
            Kind::Holiday => "holiday",
            Kind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Tentative,
    /// Readers leave it out.
    Cancelled,
    /// Done / paid; shown struck through.
    Done,
    #[default]
    #[serde(other)]
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Unique within the file, stable across rewrites.
    pub id: String,
    /// Filled in by the reader from the folder name; writers may leave it out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// `YYYY-MM-DD` or RFC 3339.
    pub start: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// IANA zone the event was made in (informational; offsets in `start` / `end` rule).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrule: Option<String>,
    /// Days (`YYYY-MM-DD`, or RFC 3339 whose local day counts) a rule skips.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exdates: Vec<String>,
    /// Groups expanded instances of one series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(default)]
    pub kind: Kind,
    /// The app's own category label, already translated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub status: Status,
    /// `#rrggbb` or a palette name (`red`, `orange`, `yellow`, `green`, `teal`, `blue`,
    /// `purple`, `pink`, `gray`); readers may ignore it and color by app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Shared entity ids (§4) it is about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<uuid::Uuid>,
    /// `otherwise://<app>/…` that opens the thing (or an https URL for web sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub busy_only: bool,
}

impl Event {
    /// An all-day event on `day`.
    pub fn day(id: impl Into<String>, kind: Kind, title: impl Into<String>, day: NaiveDate) -> Event {
        Event::new(id, kind, title, day.to_string())
    }

    pub fn new(id: impl Into<String>, kind: Kind, title: impl Into<String>, start: impl Into<String>) -> Event {
        Event { id: id.into(), app: String::new(), title: title.into(), body: None, location: None, start: start.into(), end: None, timezone: None, rrule: None, exdates: Vec::new(), series: None, kind, category: None, status: Status::Confirmed, color: None, entities: Vec::new(), link: None, busy_only: false }
    }

    pub fn end(mut self, e: impl Into<String>) -> Event {
        self.end = Some(e.into());
        self
    }

    pub fn rrule(mut self, r: impl Into<String>) -> Event {
        self.rrule = Some(r.into());
        self
    }

    pub fn body(mut self, b: impl Into<String>) -> Event {
        self.body = Some(b.into());
        self
    }

    pub fn link(mut self, l: &crate::Link) -> Event {
        self.link = Some(l.to_url());
        self
    }

    pub fn category(mut self, c: impl Into<String>) -> Event {
        self.category = Some(c.into());
        self
    }

    pub fn status(mut self, s: Status) -> Event {
        self.status = s;
        self
    }

    pub fn color(mut self, c: impl Into<String>) -> Event {
        self.color = Some(c.into());
        self
    }

    pub fn entity(mut self, id: uuid::Uuid) -> Event {
        if !self.entities.contains(&id) {
            self.entities.push(id);
        }
        self
    }

    pub fn busy_only(mut self, b: bool) -> Event {
        self.busy_only = b;
        self
    }

    pub fn all_day(&self) -> bool {
        matches!(Moment::parse(&self.start), Some(Moment::Day(_)))
    }

    /// The days and times this event happens on between `from` and `to` (local days of `tz`,
    /// inclusive); empty when `start` doesn't parse or the event is cancelled.
    pub fn occurrences<Tz: TimeZone>(&self, from: NaiveDate, to: NaiveDate, tz: &Tz) -> Vec<Occurrence> {
        expand(self, from, to, tz)
    }
}

/// A parsed `start` / `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    Day(NaiveDate),
    At(DateTime<FixedOffset>),
}

impl Moment {
    pub fn parse(s: &str) -> Option<Moment> {
        let s = s.trim();
        if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Some(Moment::Day(d));
        }
        DateTime::parse_from_rfc3339(s).ok().map(Moment::At)
    }

    /// The local calendar day in `tz`.
    pub fn day<Tz: TimeZone>(&self, tz: &Tz) -> NaiveDate {
        match self {
            Moment::Day(d) => *d,
            Moment::At(t) => t.with_timezone(tz).date_naive(),
        }
    }

    /// A day starts at its local midnight in `tz`.
    pub fn instant<Tz: TimeZone>(&self, tz: &Tz) -> DateTime<Utc> {
        match self {
            Moment::Day(d) => local_midnight(*d, tz),
            Moment::At(t) => t.with_timezone(&Utc),
        }
    }

    pub fn to_string_value(&self) -> String {
        match self {
            Moment::Day(d) => d.to_string(),
            Moment::At(t) => t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }
    }
}

fn local_midnight<Tz: TimeZone>(d: NaiveDate, tz: &Tz) -> DateTime<Utc> {
    let n = d.and_time(NaiveTime::MIN);
    tz.from_local_datetime(&n).earliest().map(|t| t.with_timezone(&Utc)).unwrap_or_else(|| n.and_utc())
}

/// One time an event happens.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub event: Event,
    pub start: Moment,
    /// All day: the last day (inclusive). Timed: the end instant (same as start if none).
    pub end: Moment,
    /// Local days it covers (inclusive).
    pub first_day: NaiveDate,
    pub last_day: NaiveDate,
}

impl Occurrence {
    pub fn all_day(&self) -> bool {
        matches!(self.start, Moment::Day(_))
    }

    /// `app/id/start`, unique across the merged calendar.
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.event.app, self.event.id, self.start.to_string_value())
    }

    pub fn covers(&self, d: NaiveDate) -> bool {
        d >= self.first_day && d <= self.last_day
    }

    /// For exports and assistants: a busy-only event loses title (→ `busy_label`), body and
    /// location; other events come back unchanged.
    pub fn redacted(&self, busy_label: &str) -> Occurrence {
        let mut o = self.clone();
        if o.event.busy_only {
            o.event.title = busy_label.to_string();
            o.event.body = None;
            o.event.location = None;
            o.event.category = None;
        }
        o
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// The RRULE subset events may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    /// Last day (inclusive) an occurrence may start on.
    pub until: Option<NaiveDate>,
    /// (n, weekday): n = 0 every such weekday, else the nth (negative from the end) of the
    /// month (MONTHLY) or of the BYMONTH month(s) (YEARLY).
    pub by_day: Vec<(i32, Weekday)>,
    pub by_month: Vec<u32>,
    pub by_month_day: Vec<i32>,
}

impl Rule {
    /// Parses `FREQ=…;…` (an optional `RRULE:` prefix is accepted). Unknown parts are ignored;
    /// an unsupported FREQ or a malformed value makes it `None`.
    pub fn parse(s: &str) -> Option<Rule> {
        let s = s.trim();
        let s = s.strip_prefix("RRULE:").unwrap_or(s);
        let mut r = Rule { freq: Freq::Daily, interval: 1, count: None, until: None, by_day: Vec::new(), by_month: Vec::new(), by_month_day: Vec::new() };
        let mut freq = None;
        for part in s.split(';').filter(|p| !p.trim().is_empty()) {
            let (k, v) = part.split_once('=')?;
            let v = v.trim();
            match k.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match v.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    })
                }
                "INTERVAL" => r.interval = v.parse().ok().filter(|n| *n > 0)?,
                "COUNT" => r.count = Some(v.parse().ok()?),
                "UNTIL" => r.until = Some(NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok().or_else(|| NaiveDate::parse_from_str(v.get(..10)?, "%Y-%m-%d").ok())?),
                "BYDAY" => r.by_day = v.split(',').map(by_day).collect::<Option<Vec<_>>>()?,
                "BYMONTH" => r.by_month = v.split(',').map(|m| m.trim().parse().ok().filter(|m| (1..=12).contains(m))).collect::<Option<Vec<_>>>()?,
                "BYMONTHDAY" => r.by_month_day = v.split(',').map(|m| m.trim().parse().ok().filter(|m: &i32| *m != 0 && (-31..=31).contains(m))).collect::<Option<Vec<_>>>()?,
                _ => {}
            }
        }
        r.freq = freq?;
        Some(r)
    }

    pub fn to_rrule(&self) -> String {
        let mut v = vec![format!(
            "FREQ={}",
            match self.freq {
                Freq::Daily => "DAILY",
                Freq::Weekly => "WEEKLY",
                Freq::Monthly => "MONTHLY",
                Freq::Yearly => "YEARLY",
            }
        )];
        if self.interval > 1 {
            v.push(format!("INTERVAL={}", self.interval));
        }
        if let Some(c) = self.count {
            v.push(format!("COUNT={c}"));
        }
        if let Some(u) = self.until {
            v.push(format!("UNTIL={}", u.format("%Y%m%d")));
        }
        if !self.by_month.is_empty() {
            v.push(format!("BYMONTH={}", self.by_month.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(",")));
        }
        if !self.by_month_day.is_empty() {
            v.push(format!("BYMONTHDAY={}", self.by_month_day.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(",")));
        }
        if !self.by_day.is_empty() {
            v.push(format!("BYDAY={}", self.by_day.iter().map(|(n, w)| format!("{}{}", if *n == 0 { String::new() } else { n.to_string() }, wd_code(*w))).collect::<Vec<_>>().join(",")));
        }
        v.join(";")
    }

    /// Start days of the occurrences from `start` up to `last` (inclusive), honoring COUNT
    /// and UNTIL, at most `cap`.
    pub fn days(&self, start: NaiveDate, last: NaiveDate, cap: usize) -> Vec<NaiveDate> {
        let end = self.until.map_or(last, |u| u.min(last));
        let mut out = Vec::new();
        if end < start {
            return out;
        }
        let interval = self.interval.max(1) as i64;
        let mut produced: u32 = 0;
        let limit = |produced: u32| self.count.is_some_and(|c| produced >= c);
        // without COUNT the periods before `from` don't matter; DAILY / WEEKLY skip ahead
        match self.freq {
            Freq::Daily => {
                let mut d = start;
                while d <= end && !limit(produced) && out.len() < cap {
                    if self.matches_filters(d) {
                        out.push(d);
                        produced += 1;
                    }
                    d += Duration::days(interval);
                }
            }
            Freq::Weekly => {
                let days: Vec<Weekday> = if self.by_day.is_empty() { vec![start.weekday()] } else { self.by_day.iter().map(|(_, w)| *w).collect() };
                let mut week = start - Duration::days(start.weekday().num_days_from_monday() as i64);
                'w: while week <= end {
                    let mut ds: Vec<NaiveDate> = days.iter().map(|w| week + Duration::days(w.num_days_from_monday() as i64)).collect();
                    ds.sort();
                    ds.dedup();
                    for d in ds {
                        if d < start || !self.matches_filters(d) {
                            continue;
                        }
                        if d > end || limit(produced) || out.len() >= cap {
                            break 'w;
                        }
                        out.push(d);
                        produced += 1;
                    }
                    week += Duration::days(7 * interval);
                }
            }
            Freq::Monthly | Freq::Yearly => {
                let (mut y, mut m) = (start.year(), start.month());
                let step_months = if self.freq == Freq::Monthly { interval } else { 12 * interval };
                'p: loop {
                    let months: Vec<u32> = if self.freq == Freq::Monthly {
                        if self.by_month.is_empty() || self.by_month.contains(&m) { vec![m] } else { vec![] }
                    } else if self.by_month.is_empty() {
                        vec![start.month()]
                    } else {
                        let mut v = self.by_month.clone();
                        v.sort();
                        v
                    };
                    if NaiveDate::from_ymd_opt(y, 1, 1).is_none_or(|d| d > end) {
                        break;
                    }
                    for mm in months {
                        for d in self.month_days(y, mm, start) {
                            if d < start {
                                continue;
                            }
                            if d > end || limit(produced) || out.len() >= cap {
                                break 'p;
                            }
                            out.push(d);
                            produced += 1;
                        }
                    }
                    let total = (y as i64) * 12 + (m as i64 - 1) + step_months;
                    y = total.div_euclid(12) as i32;
                    m = total.rem_euclid(12) as u32 + 1;
                    if y > end.year() + 1 {
                        break;
                    }
                }
            }
        }
        out
    }

    fn matches_filters(&self, d: NaiveDate) -> bool {
        (self.by_month.is_empty() || self.by_month.contains(&d.month()))
            && (self.by_month_day.is_empty() || self.by_month_day.iter().any(|md| month_day(d.year(), d.month(), *md) == Some(d)))
            && (self.freq == Freq::Weekly || self.by_day.is_empty() || self.by_day.iter().any(|(_, w)| *w == d.weekday()))
    }

    /// The days of month `m` of year `y` this rule picks, sorted.
    fn month_days(&self, y: i32, m: u32, start: NaiveDate) -> Vec<NaiveDate> {
        let mut v: Vec<NaiveDate> = if !self.by_month_day.is_empty() {
            self.by_month_day.iter().filter_map(|md| month_day(y, m, *md)).collect()
        } else if !self.by_day.is_empty() {
            self.by_day.iter().flat_map(|(n, w)| nth_weekdays(y, m, *n, *w)).collect()
        } else {
            NaiveDate::from_ymd_opt(y, m, start.day()).into_iter().collect()
        };
        if !self.by_month_day.is_empty() && !self.by_day.is_empty() {
            v.retain(|d| self.by_day.iter().any(|(_, w)| *w == d.weekday()));
        }
        v.sort();
        v.dedup();
        v
    }
}

fn wd_code(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

/// `4TH` → (4, Thursday), `-1MO` → (-1, Monday), `MO` → (0, Monday).
fn by_day(s: &str) -> Option<(i32, Weekday)> {
    let s = s.trim().to_ascii_uppercase();
    let (n, wd) = s.split_at(s.len().checked_sub(2)?);
    let wd = match wd {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    };
    let n = if n.is_empty() || n == "+" { 0 } else { n.trim_start_matches('+').parse().ok()? };
    (-5..=5).contains(&n).then_some((n, wd))
}

fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).map(|d| (d - Duration::days(1)).day()).unwrap_or(28)
}

fn month_day(y: i32, m: u32, md: i32) -> Option<NaiveDate> {
    let n = days_in_month(y, m) as i32;
    let d = if md < 0 { n + md + 1 } else { md };
    (1..=n).contains(&d).then(|| NaiveDate::from_ymd_opt(y, m, d as u32)).flatten()
}

fn nth_weekdays(y: i32, m: u32, n: i32, w: Weekday) -> Vec<NaiveDate> {
    let all: Vec<NaiveDate> = (1..=days_in_month(y, m)).filter_map(|d| NaiveDate::from_ymd_opt(y, m, d)).filter(|d| d.weekday() == w).collect();
    match n {
        0 => all,
        n if n > 0 => all.get(n as usize - 1).copied().into_iter().collect(),
        n => all.iter().rev().nth((-n) as usize - 1).copied().into_iter().collect(),
    }
}

/// See [`Event::occurrences`].
pub fn expand<Tz: TimeZone>(e: &Event, from: NaiveDate, to: NaiveDate, tz: &Tz) -> Vec<Occurrence> {
    if e.status == Status::Cancelled || to < from {
        return Vec::new();
    }
    let Some(start) = Moment::parse(&e.start) else { return Vec::new() };
    let end = e.end.as_deref().and_then(Moment::parse);
    let ex: Vec<NaiveDate> = e.exdates.iter().filter_map(|x| Moment::parse(x)).map(|m| match (m, start) {
        (Moment::Day(d), _) => d,
        (Moment::At(t), Moment::At(s)) => t.with_timezone(s.offset()).date_naive(),
        (Moment::At(t), Moment::Day(_)) => t.with_timezone(tz).date_naive(),
    }).collect();
    // the rule runs on the start's own calendar (its offset for timed events)
    let (start_day, start_time, offset) = match start {
        Moment::Day(d) => (d, None, None),
        Moment::At(t) => (t.date_naive(), Some(t.time()), Some(*t.offset())),
    };
    // length in days (all day) or seconds (timed)
    let span_days = match (start, end) {
        (Moment::Day(s), Some(Moment::Day(l))) if l >= s => (l - s).num_days().min(366),
        _ => 0,
    };
    let span_secs = match (start, end) {
        (Moment::At(s), Some(Moment::At(t))) if t >= s => (t - s).num_seconds().min(366 * 86_400),
        _ => 0,
    };
    // one day of slack either way for offsets between the event's calendar and `tz`
    let lookback = span_days + span_secs / 86_400 + 1;
    let starts: Vec<NaiveDate> = match e.rrule.as_deref().filter(|r| !r.trim().is_empty()).and_then(Rule::parse) {
        Some(rule) => {
            let lo = from - Duration::days(lookback);
            // without COUNT, whole periods before the range can be skipped (a daily rule from 1990)
            let period = match rule.freq {
                Freq::Daily => rule.interval as i64,
                Freq::Weekly => 7 * rule.interval as i64,
                _ => 0,
            };
            let first = if rule.count.is_none() && period > 0 && lo > start_day { start_day + Duration::days((lo - start_day).num_days() / period * period) } else { start_day };
            let mut ds = rule.days(first, to + Duration::days(1), 4 * MAX_OCCURRENCES);
            ds.retain(|d| *d >= lo);
            ds
        }
        None => vec![start_day],
    };
    let mut out = Vec::new();
    for d in starts {
        if ex.contains(&d) {
            continue;
        }
        let (s, en) = match (start_time, offset) {
            (Some(t), Some(off)) => {
                let Some(s) = off.from_local_datetime(&NaiveDateTime::new(d, t)).single() else { continue };
                (Moment::At(s), Moment::At(s + Duration::seconds(span_secs)))
            }
            _ => (Moment::Day(d), Moment::Day(d + Duration::days(span_days))),
        };
        let first = s.day(tz);
        let last = match en {
            Moment::Day(l) => l,
            // an end at exactly midnight doesn't cover the next day
            Moment::At(t) if span_secs > 0 => (t - Duration::seconds(1)).with_timezone(tz).date_naive().max(first),
            Moment::At(_) => first,
        };
        if last < from || first > to {
            continue;
        }
        out.push(Occurrence { event: e.clone(), start: s, end: en, first_day: first, last_day: last });
        if out.len() >= MAX_OCCURRENCES {
            break;
        }
    }
    out
}

/// Sorts occurrences: by first day, all-day ones first, then by start time, app, id.
pub fn sort<Tz: TimeZone>(v: &mut [Occurrence], tz: &Tz) {
    v.sort_by(|a, b| {
        a.first_day
            .cmp(&b.first_day)
            .then(b.all_day().cmp(&a.all_day()))
            .then_with(|| a.start.instant(tz).cmp(&b.start.instant(tz)))
            .then_with(|| a.event.app.cmp(&b.event.app))
            .then_with(|| a.event.id.cmp(&b.event.id))
    });
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub schema: u32,
    pub app: String,
    #[serde(default)]
    pub updated_at: String,
    /// The range an app that wrote expanded instances covered (`YYYY-MM-DD`, inclusive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    #[serde(default)]
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

pub fn feed_path(root: &Root, app: &str) -> PathBuf {
    root.app_dir(app).join(FILE)
}

/// Replaces `app`'s events file with `events` (first [`MAX_EVENTS`] with a unique id and a
/// parsable start, titles cut, `app` filled in).
pub fn publish(root: &Root, app: &str, events: &[Event], window: Option<Window>) -> std::io::Result<()> {
    if !valid_app_id(app) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("invalid app id {app:?}")));
    }
    let mut seen = std::collections::BTreeSet::new();
    let events: Vec<Event> = events
        .iter()
        .filter(|e| !e.id.is_empty() && Moment::parse(&e.start).is_some() && seen.insert(e.id.clone()))
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
    let feed = Feed { schema: SCHEMA, app: app.to_string(), updated_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true), window, events };
    let bytes = serde_json::to_vec_pretty(&feed).unwrap_or_default();
    // rewriting an unchanged list would wake every reader (and the sync client) for nothing
    if let Ok(old) = std::fs::read(feed_path(root, app))
        && let Ok(old) = serde_json::from_slice::<Feed>(&old)
        && old.events == feed.events
        && old.window == feed.window
        && old.schema == SCHEMA
    {
        return Ok(());
    }
    crate::fsutil::write_atomic(&feed_path(root, app), &bytes)
}

/// Removes `app`'s events file (the user turned its calendar off).
pub fn clear(root: &Root, app: &str) -> std::io::Result<()> {
    match std::fs::remove_file(feed_path(root, app)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Feeds {
    pub feeds: Vec<Feed>,
    /// Files that couldn't be read: (app id, reason).
    pub errors: Vec<(String, String)>,
}

/// Every app's events file under `root` (folders named by a valid app id holding `events.json`).
pub fn feeds(root: &Root) -> Feeds {
    let mut out = Feeds::default();
    let Ok(dir) = std::fs::read_dir(root.dir()) else { return out };
    let mut apps: Vec<String> = dir.filter_map(|e| e.ok()).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).filter_map(|e| e.file_name().to_str().map(str::to_string)).filter(|a| valid_app_id(a)).collect();
    apps.sort();
    for app in apps {
        let path = feed_path(root, &app);
        let Ok(md) = std::fs::metadata(&path) else { continue };
        if md.len() > MAX_FILE {
            out.errors.push((app, format!("larger than {MAX_FILE} bytes")));
            continue;
        }
        let parsed = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Feed>(&s).map_err(|e| e.to_string()));
        match parsed {
            Ok(f) if f.schema > SCHEMA => out.errors.push((app, format!("schema {} is newer than {SCHEMA}", f.schema))),
            Ok(mut f) => {
                f.events.truncate(MAX_EVENTS);
                for e in &mut f.events {
                    // the folder is authoritative: an app can't post as another
                    e.app = app.clone();
                }
                f.app = app;
                out.feeds.push(f);
            }
            Err(e) => out.errors.push((app, e)),
        }
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Merged {
    pub occurrences: Vec<Occurrence>,
    /// app id → when its file was written.
    pub updated: BTreeMap<String, String>,
    pub errors: Vec<(String, String)>,
}

/// Every app's occurrences between `from` and `to` (local days, inclusive), sorted.
pub fn read(root: &Root, from: NaiveDate, to: NaiveDate) -> Merged {
    read_in(root, from, to, &Local)
}

pub fn read_in<Tz: TimeZone>(root: &Root, from: NaiveDate, to: NaiveDate, tz: &Tz) -> Merged {
    let f = feeds(root);
    let mut m = Merged { errors: f.errors, ..Default::default() };
    for feed in f.feeds {
        m.updated.insert(feed.app.clone(), feed.updated_at.clone());
        m.occurrences.extend(feed.events.iter().flat_map(|e| expand(e, from, to, tz)));
    }
    sort(&mut m.occurrences, tz);
    m
}

/// Occurrences of `events` (any source: feeds, ICS overlays, holidays) in the range, sorted.
pub fn expand_all<Tz: TimeZone>(events: &[Event], from: NaiveDate, to: NaiveDate, tz: &Tz) -> Vec<Occurrence> {
    let mut v: Vec<Occurrence> = events.iter().flat_map(|e| expand(e, from, to, tz)).collect();
    sort(&mut v, tz);
    v
}

/// Holidays of `countries` as all-day events of app `holidays` (no link), for merging.
pub fn holiday_events(countries: &[crate::holidays::Country], from: NaiveDate, to: NaiveDate) -> Vec<Event> {
    crate::holidays::between(countries, from, to)
        .into_iter()
        .map(|h| {
            let mut e = Event::day(format!("{}-{}", h.id, h.date), Kind::Holiday, h.name, h.date);
            e.app = HOLIDAYS_APP.to_string();
            e.category = Some(h.country.id().to_string());
            e
        })
        .collect()
}

/// The pseudo app id holidays are merged under.
pub const HOLIDAYS_APP: &str = "holidays";

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn days(r: &str, start: &str, last: &str) -> Vec<String> {
        Rule::parse(r).unwrap().days(d(start), d(last), 100).into_iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn rules() {
        assert_eq!(days("FREQ=YEARLY", "2020-12-25", "2022-12-31"), ["2020-12-25", "2021-12-25", "2022-12-25"]);
        assert_eq!(days("FREQ=YEARLY;BYMONTH=11;BYDAY=4TH", "2021-11-25", "2023-12-31"), ["2021-11-25", "2022-11-24", "2023-11-23"]);
        // Feb 29 only in leap years
        assert_eq!(days("FREQ=YEARLY", "2024-02-29", "2032-12-31"), ["2024-02-29", "2028-02-29", "2032-02-29"]);
        // the 31st skips short months; BYMONTHDAY=-1 is every month end
        assert_eq!(days("FREQ=MONTHLY", "2026-01-31", "2026-05-31"), ["2026-01-31", "2026-03-31", "2026-05-31"]);
        assert_eq!(days("FREQ=MONTHLY;BYMONTHDAY=-1", "2026-01-31", "2026-04-30"), ["2026-01-31", "2026-02-28", "2026-03-31", "2026-04-30"]);
        assert_eq!(days("FREQ=MONTHLY;INTERVAL=3;COUNT=3", "2026-01-05", "2030-01-01"), ["2026-01-05", "2026-04-05", "2026-07-05"]);
        assert_eq!(days("FREQ=MONTHLY;BYDAY=-1FR;COUNT=2", "2026-11-01", "2027-12-31"), ["2026-11-27", "2026-12-25"]);
        assert_eq!(days("FREQ=MONTHLY;BYMONTHDAY=15,30", "2026-02-01", "2026-03-31"), ["2026-02-15", "2026-03-15", "2026-03-30"]);
        assert_eq!(days("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4", "2026-11-18", "2027-01-01"), ["2026-11-18", "2026-11-23", "2026-11-25", "2026-11-30"]);
        assert_eq!(days("FREQ=WEEKLY;INTERVAL=2;UNTIL=20261231", "2026-11-02", "2027-12-31"), ["2026-11-02", "2026-11-16", "2026-11-30", "2026-12-14", "2026-12-28"]);
        assert_eq!(days("FREQ=DAILY;COUNT=3", "2026-11-30", "2027-01-01"), ["2026-11-30", "2026-12-01", "2026-12-02"]);
        assert_eq!(days("FREQ=DAILY;BYDAY=MO", "2026-11-20", "2026-12-01"), ["2026-11-23", "2026-11-30"]);
        assert!(Rule::parse("FREQ=HOURLY").is_none());
        assert!(Rule::parse("INTERVAL=2").is_none());
        assert!(Rule::parse("FREQ=DAILY;INTERVAL=0").is_none());
        for r in ["FREQ=YEARLY;BYMONTH=11;BYDAY=4TH", "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR", "FREQ=MONTHLY;COUNT=3;BYMONTHDAY=-1", "FREQ=DAILY;UNTIL=20270101"] {
            assert_eq!(Rule::parse(r).unwrap().to_rrule(), r);
        }
    }

    #[test]
    fn expands_in_a_range() {
        let utc = Utc;
        let bday = Event::day("p1-bday", Kind::Birthday, "Ana", d("1990-05-12")).rrule("FREQ=YEARLY");
        let o = bday.occurrences(d("2026-01-01"), d("2027-12-31"), &utc);
        assert_eq!(o.iter().map(|o| o.first_day.to_string()).collect::<Vec<_>>(), ["2026-05-12", "2027-05-12"]);
        // a multi-day event that started before the range still shows
        let trip = Event::day("t", Kind::Other, "Trip", d("2026-11-28")).end("2026-12-02");
        let o = trip.occurrences(d("2026-12-01"), d("2026-12-31"), &utc);
        assert_eq!(o.len(), 1);
        assert!(o[0].covers(d("2026-12-02")) && !o[0].covers(d("2026-12-03")));
        // timed, weekly, with an exdate; the time keeps its offset
        let mtg = Event::new("m", Kind::Meeting, "Standup", "2026-11-02T09:00:00-05:00").end("2026-11-02T09:30:00-05:00").rrule("FREQ=WEEKLY").busy_only(true);
        let mut mtg = mtg;
        mtg.exdates = vec!["2026-11-09".into()];
        let o = mtg.occurrences(d("2026-11-01"), d("2026-11-20"), &FixedOffset::west_opt(5 * 3600).unwrap());
        let starts: Vec<String> = o.iter().map(|o| o.start.to_string_value()).collect();
        assert_eq!(starts, ["2026-11-02T09:00:00-05:00", "2026-11-16T09:00:00-05:00"]);
        assert_eq!(o[0].end.to_string_value(), "2026-11-02T09:30:00-05:00");
        let r = o[0].redacted("Busy");
        assert_eq!((r.event.title.as_str(), r.event.body.is_none()), ("Busy", true));
        // a late-evening UTC event lands on the previous local day in Bogotá
        let late = Event::new("l", Kind::Other, "x", "2026-11-21T02:00:00Z");
        let o = late.occurrences(d("2026-11-20"), d("2026-11-20"), &FixedOffset::west_opt(5 * 3600).unwrap());
        assert_eq!(o.len(), 1);
        // cancelled and unparsable events produce nothing
        assert!(Event::day("c", Kind::Other, "c", d("2026-11-21")).status(Status::Cancelled).occurrences(d("2026-11-01"), d("2026-11-30"), &utc).is_empty());
        assert!(Event::new("b", Kind::Other, "b", "soon").occurrences(d("2026-11-01"), d("2026-11-30"), &utc).is_empty());
        // daily rules started long ago are capped, not walked forever
        let daily = Event::day("dd", Kind::Other, "d", d("1900-01-01")).rrule("FREQ=DAILY");
        assert_eq!(daily.occurrences(d("2026-01-01"), d("2026-01-31"), &utc).len(), 31);
    }

    #[test]
    fn publish_and_read_merge_feeds() {
        let dir = tempfile::tempdir().unwrap();
        let root = Root::at(dir.path());
        let v = vec![
            Event::day("a", Kind::Due, "SOAT", d("2026-11-30")).link(&crate::Link::new("vigia").segment("item").segment("1")),
            Event::day("a", Kind::Due, "duplicate id", d("2026-11-30")),
            Event::new("bad", Kind::Due, "no date", "tomorrow"),
        ];
        publish(&root, "vigia", &v, None).unwrap();
        publish(&root, "expense", &[Event::new("p", Kind::Payment, "Rent", "2026-11-01").rrule("FREQ=MONTHLY").busy_only(true)], None).unwrap();
        // a file posing as another app is read under its folder's id
        std::fs::create_dir_all(dir.path().join("rogue")).unwrap();
        std::fs::write(dir.path().join("rogue/events.json"), r#"{"schema":1,"app":"vigia","events":[{"id":"x","title":"t","start":"2026-11-15","kind":"weird"}]}"#).unwrap();
        std::fs::create_dir_all(dir.path().join("future")).unwrap();
        std::fs::write(dir.path().join("future/events.json"), r#"{"schema":2,"app":"future","events":[]}"#).unwrap();
        std::fs::create_dir_all(dir.path().join("broken")).unwrap();
        std::fs::write(dir.path().join("broken/events.json"), "{").unwrap();
        let m = read_in(&root, d("2026-11-01"), d("2026-12-31"), &Utc);
        let keys: Vec<String> = m.occurrences.iter().map(|o| o.key()).collect();
        assert_eq!(keys, ["expense/p/2026-11-01", "rogue/x/2026-11-15", "vigia/a/2026-11-30", "expense/p/2026-12-01"]);
        assert_eq!(m.occurrences[1].event.kind, Kind::Other);
        assert_eq!(m.errors.iter().map(|(a, _)| a.as_str()).collect::<Vec<_>>(), ["broken", "future"]);
        assert!(m.updated.contains_key("vigia"));
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(feed_path(&root, "expense")).unwrap()).unwrap();
        assert_eq!(raw["events"][0]["busyOnly"], serde_json::json!(true));
        assert!(raw["events"][0].get("exdates").is_none());
        // an unchanged list isn't rewritten; a changed one is, leaving no temp files
        let before = std::fs::read_to_string(feed_path(&root, "vigia")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        publish(&root, "vigia", &v, None).unwrap();
        assert_eq!(std::fs::read_to_string(feed_path(&root, "vigia")).unwrap(), before);
        publish(&root, "vigia", &[], None).unwrap();
        assert_eq!(std::fs::read_dir(dir.path().join("vigia")).unwrap().count(), 1);
        clear(&root, "vigia").unwrap();
        clear(&root, "vigia").unwrap();
        assert!(!feed_path(&root, "vigia").exists());
        assert!(publish(&root, "Bad App", &[], None).is_err());
    }

    #[test]
    fn holidays_merge_as_events() {
        let h = holiday_events(&[crate::holidays::Country::Co], d("2026-12-01"), d("2026-12-31"));
        assert_eq!(h.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["co_immaculate-2026-12-08", "co_christmas-2026-12-25"]);
        assert!(h.iter().all(|e| e.app == HOLIDAYS_APP && e.kind == Kind::Holiday && e.all_day()));
    }
}
