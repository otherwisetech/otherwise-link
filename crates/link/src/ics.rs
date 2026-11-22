//! iCalendar (RFC 5545) for the events feed: [`write`] turns events into a calendar other apps
//! (Apple Calendar, Outlook, Google) can import or subscribe to, and [`parse`] reads a calendar
//! (a file or a downloaded URL) into [`Event`]s so overlays expand with the same rules.
//!
//! Parsing covers what calendars publish in practice: all-day and timed `DTSTART` / `DTEND` /
//! `DURATION`, `RRULE` (the subset [`crate::events::Rule`] supports; events with other rules
//! keep only their first occurrence), `EXDATE`, `RECURRENCE-ID` overrides, `STATUS`,
//! `SUMMARY`, `DESCRIPTION`, `LOCATION`, `CATEGORIES`, `URL`, `X-WR-CALNAME`. Times with a
//! `TZID` or without a zone are read as wall-clock times of the zone the caller passes (no
//! time-zone database); `Z` times are exact.
use crate::events::{Event, Kind, Moment, Rule, Status};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};

pub const MAX_INPUT: usize = 16 << 20;
pub const MAX_PARSED: usize = 20_000;

pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace('\n', "\\n")
}

pub fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(x) => out.push(x),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Lines longer than 75 octets continue on the next line after a space (§3.1).
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in line.chars() {
        let w = c.len_utf8();
        if n + w > 75 {
            out.push_str("\r\n ");
            n = 1;
        }
        out.push(c);
        n += w;
    }
    out.push_str("\r\n");
    out
}

fn ics_day(d: NaiveDate) -> String {
    d.format("%Y%m%d").to_string()
}

fn ics_utc(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Options for [`write`].
#[derive(Debug, Clone)]
pub struct Options {
    /// `X-WR-CALNAME`.
    pub name: String,
    /// `PRODID` product part (`Otherwise Apps`).
    pub product: String,
    /// Replaces the title of busy-only events ("Busy"); their details are left out.
    pub busy_label: String,
    /// `DTSTAMP` of every event.
    pub stamp: DateTime<Utc>,
    /// Suggested refresh for subscribers.
    pub refresh_hours: u32,
}

impl Default for Options {
    fn default() -> Options {
        Options { name: "Otherwise".into(), product: "Otherwise".into(), busy_label: "Busy".into(), stamp: Utc::now(), refresh_hours: 6 }
    }
}

/// `<id>.<app>@otherwise.local` (stable, so subscribers update instead of duplicating).
pub fn uid(e: &Event) -> String {
    let clean = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '-' }).collect::<String>();
    let app = if e.app.is_empty() { "app".to_string() } else { clean(&e.app) };
    format!("{}.{app}@otherwise.local", clean(&e.id))
}

/// A whole calendar for `events` (cancelled ones left out; rules and exdates kept).
pub fn write(events: &[Event], o: &Options) -> String {
    let mut out = String::new();
    let mut line = |s: &str| out.push_str(&fold(s));
    line("BEGIN:VCALENDAR");
    line("VERSION:2.0");
    line(&format!("PRODID:-//Otherwise Technologies LLC//{}//EN", escape(&o.product)));
    line("CALSCALE:GREGORIAN");
    line("METHOD:PUBLISH");
    line(&format!("X-WR-CALNAME:{}", escape(&o.name)));
    line(&format!("REFRESH-INTERVAL;VALUE=DURATION:PT{}H", o.refresh_hours.max(1)));
    line(&format!("X-PUBLISHED-TTL:PT{}H", o.refresh_hours.max(1)));
    for e in events.iter().filter(|e| e.status != Status::Cancelled) {
        let Some(start) = Moment::parse(&e.start) else { continue };
        line("BEGIN:VEVENT");
        line(&format!("UID:{}", uid(e)));
        line(&format!("DTSTAMP:{}", ics_utc(o.stamp)));
        match start {
            Moment::Day(d) => {
                let last = match e.end.as_deref().and_then(Moment::parse) {
                    Some(Moment::Day(l)) if l >= d => l,
                    _ => d,
                };
                line(&format!("DTSTART;VALUE=DATE:{}", ics_day(d)));
                line(&format!("DTEND;VALUE=DATE:{}", ics_day(last + Duration::days(1))));
            }
            Moment::At(t) => {
                line(&format!("DTSTART:{}", ics_utc(t.with_timezone(&Utc))));
                if let Some(Moment::At(en)) = e.end.as_deref().and_then(Moment::parse).filter(|en| matches!(en, Moment::At(x) if *x >= t)) {
                    line(&format!("DTEND:{}", ics_utc(en.with_timezone(&Utc))));
                }
            }
        }
        if let Some(r) = e.rrule.as_deref().and_then(Rule::parse) {
            line(&format!("RRULE:{}", r.to_rrule()));
        }
        for x in e.exdates.iter().filter_map(|x| Moment::parse(x)) {
            match (x, start) {
                (Moment::Day(d), Moment::Day(_)) => line(&format!("EXDATE;VALUE=DATE:{}", ics_day(d))),
                (Moment::Day(d), Moment::At(t)) => {
                    if let Some(at) = t.offset().from_local_datetime(&NaiveDateTime::new(d, t.time())).single() {
                        line(&format!("EXDATE:{}", ics_utc(at.with_timezone(&Utc))));
                    }
                }
                (Moment::At(at), _) => line(&format!("EXDATE:{}", ics_utc(at.with_timezone(&Utc)))),
            }
        }
        if e.busy_only {
            line(&format!("SUMMARY:{}", escape(&o.busy_label)));
            line("CLASS:PRIVATE");
        } else {
            line(&format!("SUMMARY:{}", escape(&e.title)));
            if let Some(b) = e.body.as_deref().filter(|b| !b.is_empty()) {
                line(&format!("DESCRIPTION:{}", escape(b)));
            }
            if let Some(l) = e.location.as_deref().filter(|l| !l.is_empty()) {
                line(&format!("LOCATION:{}", escape(l)));
            }
            if let Some(c) = e.category.as_deref().filter(|c| !c.is_empty()) {
                line(&format!("CATEGORIES:{}", escape(c)));
            }
        }
        if let Some(l) = e.link.as_deref().filter(|l| !l.contains(char::is_whitespace)) {
            line(&format!("URL:{l}"));
        }
        if e.status == Status::Tentative {
            line("STATUS:TENTATIVE");
        }
        // deadlines and reminders don't make the user busy; meetings and bookings do
        let opaque = matches!(e.kind, Kind::Meeting | Kind::Booking | Kind::Audit | Kind::Maintenance);
        line(if opaque { "TRANSP:OPAQUE" } else { "TRANSP:TRANSPARENT" });
        line("END:VEVENT");
    }
    line("END:VCALENDAR");
    out
}

/// What [`parse`] found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Calendar {
    /// `X-WR-CALNAME`.
    pub name: Option<String>,
    pub events: Vec<Event>,
}

/// Whether the text is a calendar at all.
pub fn looks_like_ics(body: &str) -> bool {
    body.trim_start_matches('\u{feff}').trim_start().starts_with("BEGIN:VCALENDAR")
}

/// `webcal://` is https in disguise.
pub fn fetch_url(s: &str) -> String {
    let t = s.trim();
    match t.get(..9) {
        Some(p) if p.eq_ignore_ascii_case("webcal://") => format!("https://{}", &t[9..]),
        _ => t.to_string(),
    }
}

fn param<'a>(name: &'a str, key: &str) -> Option<&'a str> {
    name.split(';').skip(1).filter_map(|p| p.split_once('=')).find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.trim_matches('"'))
}

/// A `DTSTART`-like value: `VALUE=DATE` / 8 digits → a day; `…Z` → UTC; otherwise wall-clock
/// time in `tz`.
fn moment<Tz: TimeZone>(name: &str, value: &str, tz: &Tz) -> Option<Moment> {
    let v = value.trim();
    if param(name, "VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE")) || v.len() == 8 {
        return NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok().map(Moment::Day);
    }
    let (body, utc) = match v.strip_suffix('Z').or_else(|| v.strip_suffix('z')) {
        Some(b) => (b, true),
        None => (v, false),
    };
    let n = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S").or_else(|_| NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M")).ok()?;
    let t: DateTime<FixedOffset> = if utc {
        n.and_utc().fixed_offset()
    } else {
        let l = tz.from_local_datetime(&n).earliest()?;
        l.fixed_offset()
    };
    Some(Moment::At(t))
}

/// `P1D`, `PT1H30M`, `-P2W`… (sign ignored: durations of events are positive).
fn duration(s: &str) -> Option<Duration> {
    let s = s.trim().trim_start_matches(['+', '-']);
    let s = s.strip_prefix('P')?;
    let (date, time) = s.split_once('T').unwrap_or((s, ""));
    let mut total = Duration::zero();
    let mut num = String::new();
    for c in date.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: i64 = num.parse().ok()?;
        num.clear();
        total += match c {
            'W' => Duration::weeks(n),
            'D' => Duration::days(n),
            _ => return None,
        };
    }
    for c in time.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: i64 = num.parse().ok()?;
        num.clear();
        total += match c {
            'H' => Duration::hours(n),
            'M' => Duration::minutes(n),
            'S' => Duration::seconds(n),
            _ => return None,
        };
    }
    Some(total)
}

#[derive(Default)]
struct Raw {
    uid: String,
    summary: String,
    description: String,
    location: String,
    url: String,
    categories: String,
    start: Option<Moment>,
    end: Option<Moment>,
    duration: Option<Duration>,
    rrule: String,
    exdates: Vec<Moment>,
    recurrence_id: Option<Moment>,
    status: Status,
}

/// Events of `text` (all of `VEVENT`s; others ignored), with ids from `UID` (+ the
/// occurrence for overrides). Times without a zone are read in `tz`.
pub fn parse<Tz: TimeZone>(text: &str, tz: &Tz) -> Calendar {
    let text = if text.len() > MAX_INPUT { text.get(..MAX_INPUT).unwrap_or("") } else { text };
    let mut lines: Vec<String> = Vec::new();
    for raw in text.trim_start_matches('\u{feff}').split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
            }
        } else {
            lines.push(raw.to_string());
        }
    }
    let mut cal = Calendar::default();
    let mut raws: Vec<Raw> = Vec::new();
    let mut cur: Option<Raw> = None;
    // nested components (VALARM) inside an event
    let mut depth = 0usize;
    for l in lines {
        let Some((name, value)) = l.split_once(':') else { continue };
        let key = name.split(';').next().unwrap_or(name).trim().to_ascii_uppercase();
        match (key.as_str(), value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if cur.is_none() => {
                cur = Some(Raw::default());
                depth = 0;
                continue;
            }
            ("END", "VEVENT") => {
                if let Some(r) = cur.take()
                    && raws.len() < MAX_PARSED
                {
                    raws.push(r);
                }
                continue;
            }
            ("BEGIN", _) if cur.is_some() => {
                depth += 1;
                continue;
            }
            ("END", _) if cur.is_some() => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        let Some(r) = cur.as_mut() else {
            if key == "X-WR-CALNAME" && cal.name.is_none() {
                cal.name = Some(unescape(value.trim())).filter(|n| !n.is_empty());
            }
            continue;
        };
        if depth > 0 {
            continue;
        }
        match key.as_str() {
            "UID" => r.uid = value.trim().to_string(),
            "SUMMARY" => r.summary = unescape(value),
            "DESCRIPTION" => r.description = unescape(value),
            "LOCATION" => r.location = unescape(value),
            "URL" => r.url = value.trim().to_string(),
            "CATEGORIES" => r.categories = unescape(value),
            "DTSTART" => r.start = moment(name, value, tz),
            "DTEND" => r.end = moment(name, value, tz),
            "DURATION" => r.duration = duration(value),
            "RRULE" => r.rrule = value.trim().to_string(),
            "EXDATE" => r.exdates.extend(value.split(',').filter_map(|v| moment(name, v, tz))),
            "RECURRENCE-ID" => r.recurrence_id = moment(name, value, tz),
            "STATUS" => {
                r.status = match value.trim().to_ascii_uppercase().as_str() {
                    "CANCELLED" => Status::Cancelled,
                    "TENTATIVE" => Status::Tentative,
                    _ => Status::Confirmed,
                }
            }
            _ => {}
        }
    }
    // an override (RECURRENCE-ID) replaces that occurrence of its series
    let overrides: Vec<(String, Moment)> = raws.iter().filter_map(|r| r.recurrence_id.map(|m| (r.uid.clone(), m))).collect();
    for (n, r) in raws.into_iter().enumerate() {
        let Some(start) = r.start else { continue };
        let all_day = matches!(start, Moment::Day(_));
        let end = match (start, r.end, r.duration) {
            // DTEND of an all-day event is exclusive; ours is the last day
            (Moment::Day(s), Some(Moment::Day(e)), _) => Some(Moment::Day((e - Duration::days(1)).max(s))),
            (Moment::Day(s), None, Some(d)) if d.num_days() > 1 => Some(Moment::Day(s + Duration::days(d.num_days() - 1))),
            (Moment::At(s), Some(Moment::At(e)), _) if e > s => Some(Moment::At(e)),
            (Moment::At(s), None, Some(d)) if d > Duration::zero() => Some(Moment::At(s + d)),
            _ => None,
        };
        let id = match (&r.recurrence_id, r.uid.is_empty()) {
            (Some(m), false) => format!("{}#{}", r.uid, m.to_string_value()),
            (None, false) => r.uid.clone(),
            (_, true) => format!("event-{n}"),
        };
        let mut e = Event::new(id, Kind::Other, if r.summary.trim().is_empty() { String::new() } else { r.summary.trim().to_string() }, start.to_string_value());
        e.end = end.map(|m| m.to_string_value());
        e.body = Some(r.description.trim().to_string()).filter(|s| !s.is_empty());
        e.location = Some(r.location.trim().to_string()).filter(|s| !s.is_empty());
        e.category = r.categories.split(',').map(str::trim).find(|s| !s.is_empty()).map(str::to_string);
        e.link = Some(r.url.clone()).filter(|u| !u.is_empty());
        e.status = r.status;
        if r.recurrence_id.is_none() {
            e.rrule = Rule::parse(&r.rrule).map(|x| x.to_rrule());
            let mut ex: Vec<Moment> = r.exdates.clone();
            ex.extend(overrides.iter().filter(|(u, _)| *u == r.uid && !u.is_empty()).map(|(_, m)| *m));
            e.exdates = ex.iter().map(|m| match (m, start) {
                (Moment::At(t), Moment::At(s)) => t.with_timezone(s.offset()).date_naive().to_string(),
                (m, _) => m.to_string_value(),
            }).collect();
        }
        if all_day {
            e.timezone = None;
        }
        cal.events.push(e);
    }
    cal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Occurrence;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn writes_and_parses_back() {
        let mut a = Event::day("item-7", Kind::Due, "Pasaporte; Ana, con \\ y una línea muy larga que obliga a doblar la línea del archivo iCalendar", d("2031-05-12")).body("Consulado\nsegunda línea").link(&crate::Link::new("vigia").segment("item").segment("7"));
        a.app = "vigia".into();
        let mut b = Event::new("pay", Kind::Payment, "Rent 4.500.000", "2026-11-05T14:00:00Z").end("2026-11-05T15:00:00Z").rrule("FREQ=MONTHLY;COUNT=12").busy_only(true).body("amount");
        b.app = "expense".into();
        b.exdates = vec!["2026-12-05".into()];
        let mut trip = Event::day("trip", Kind::Other, "Feria", d("2026-08-03")).end("2026-08-05");
        trip.app = "x".into();
        let gone = Event::day("gone", Kind::Other, "gone", d("2026-01-01")).status(Status::Cancelled);
        let o = Options { name: "Otherwise".into(), stamp: DateTime::from_timestamp(1_790_000_000, 0).unwrap(), ..Default::default() };
        let text = write(&[a.clone(), b, trip, gone], &o);
        assert!(text.lines().all(|l| l.len() <= 76), "folded");
        assert!(text.ends_with("END:VCALENDAR\r\n") && !text.contains("gone"));
        assert!(!text.contains("4.500.000") && !text.contains("amount") && text.contains("SUMMARY:Busy\r\nCLASS:PRIVATE"));
        assert!(text.contains("URL:otherwise://vigia/item/7"));
        let cal = parse(&text, &Utc);
        assert_eq!(cal.name.as_deref(), Some("Otherwise"));
        assert_eq!(cal.events.len(), 3);
        assert_eq!(cal.events[0].id, "item-7.vigia@otherwise.local");
        assert_eq!(cal.events[0].title, a.title);
        assert_eq!(cal.events[0].body.as_deref(), Some("Consulado\nsegunda línea"));
        assert_eq!((cal.events[0].start.as_str(), cal.events[0].end.as_deref()), ("2031-05-12", Some("2031-05-12")));
        assert_eq!(cal.events[0].link.as_deref(), Some("otherwise://vigia/item/7"));
        assert_eq!(cal.events[1].rrule.as_deref(), Some("FREQ=MONTHLY;COUNT=12"));
        assert_eq!(cal.events[1].exdates, ["2026-12-05"]);
        assert_eq!(cal.events[1].end.as_deref(), Some("2026-11-05T15:00:00Z"));
        let occ = cal.events[1].occurrences(d("2026-11-01"), d("2027-01-31"), &Utc);
        assert_eq!(occ.iter().map(|o| o.first_day.to_string()).collect::<Vec<_>>(), ["2026-11-05", "2027-01-05"]);
        assert_eq!(cal.events[2].end.as_deref(), Some("2026-08-05"));
    }

    #[test]
    fn reads_what_calendars_publish() {
        let text = "\u{feff}BEGIN:VCALENDAR\r\nX-WR-CALNAME:Equipo\r\nBEGIN:VTIMEZONE\r\nTZID:America/Bogota\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:w1\r\nSUMMARY:Weekly\r\nDTSTART;TZID=America/Bogota:20261102T090000\r\nDURATION:PT45M\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEXDATE;TZID=America/Bogota:20261109T090000\r\nBEGIN:VALARM\r\nTRIGGER:-PT10M\r\nDESCRIPTION:alarm text\r\nEND:VALARM\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:w1\r\nRECURRENCE-ID;TZID=America/Bogota:20261116T090000\r\nSUMMARY:Weekly (moved)\r\nDTSTART;TZID=America/Bogota:20261117T100000\r\nDTEND;TZID=America/Bogota:20261117T110000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Cancelled\r\nSTATUS:CANCELLED\r\nDTSTART;VALUE=DATE:20261120\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:h\r\nSUMMARY:Feria\\, larga\r\nCATEGORIES:Fiestas,Otros\r\nDTSTART;VALUE=DATE:20261201\r\nDTEND;VALUE=DATE:20261204\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:no uid\r\nDTSTART:20261125T120000Z\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:nostart\r\nSUMMARY:broken\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let bogota = FixedOffset::west_opt(5 * 3600).unwrap();
        let cal = parse(text, &bogota);
        assert_eq!(cal.name.as_deref(), Some("Equipo"));
        assert_eq!(cal.events.len(), 5);
        let occ: Vec<Occurrence> = crate::events::expand_all(&cal.events, d("2026-11-01"), d("2026-12-31"), &bogota);
        let got: Vec<(String, String)> = occ.iter().filter(|o| o.event.id.starts_with("w1")).map(|o| (o.first_day.to_string(), o.event.title.clone())).take(4).collect();
        assert_eq!(got, [("2026-11-02".into(), "Weekly".into()), ("2026-11-17".into(), "Weekly (moved)".into()), ("2026-11-23".into(), "Weekly".into()), ("2026-11-30".into(), "Weekly".into())]);
        assert_eq!(occ[0].end.to_string_value(), "2026-11-02T09:45:00-05:00");
        assert!(occ.iter().all(|o| o.event.title != "Cancelled" && o.event.title != "alarm text"));
        let feria = occ.iter().find(|o| o.event.id == "h").unwrap();
        assert_eq!((feria.event.title.as_str(), feria.event.category.as_deref(), feria.last_day), ("Feria, larga", Some("Fiestas"), d("2026-12-03")));
        assert!(cal.events.iter().any(|e| e.id.starts_with("event-") && e.start == "2026-11-25T12:00:00Z"));
        assert_eq!(fetch_url("webcal://example.com/a.ics"), "https://example.com/a.ics");
        assert!(looks_like_ics(text) && !looks_like_ics("<html>"));
        assert_eq!(duration("P1W2DT3H"), Some(Duration::days(9) + Duration::hours(3)));
    }
}
