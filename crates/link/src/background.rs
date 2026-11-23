//! Background jobs: work an installed app wants done while none of its windows is open (check
//! mail, ping devices, refresh calendars), run by Otherwise Helper, the launcher's small
//! menu-bar process.
//!
//! **Declared inside the app's own install**, never in the data root: a macOS bundle ships
//! `Contents/Resources/otherwise-background.json`, a Windows / Linux install folder ships
//! `otherwise-background.json` next to its binary. The helper finds jobs by scanning installed
//! apps ([`scan`]): no first launch needed, nothing to register, and deleting the app makes its
//! jobs disappear on the next scan. A stale file some older version left in `<root>/<app>/` is
//! never read.
//!
//! ```json
//! { "schema": 1, "app": "expense", "jobs": [
//!   { "id": "mail", "label": { "key": "background.mail", "en": "Check mail", "es": "Revisar correo" },
//!     "every": "15m", "exe": "expense", "args": ["background", "run", "mail"],
//!     "timeoutSecs": 300, "memoryMb": 400, "whileOpen": true, "heavy": true, "network": true } ] }
//! ```
//!
//! - `exe`: a file name in the bundle's `Contents/MacOS` (Windows / Linux: in the install
//!   folder, `.exe` added on Windows); default the app's GUI binary. Paths and `..` are refused:
//!   the helper only ever runs the app's own binary ([`plan`] re-checks the install every time).
//! - `every` (`30s`, `15m`, `1h`, `1d`) or `daily` (`HH:MM`, local time).
//! - `whileOpen`: run even while the app has a window open (default true). A job that writes
//!   MUST go through the app when it is open (its socket), else write under [`crate::lock`].
//! - `heavy`: uses real CPU / memory / network; the helper never runs two at once and skips them
//!   under memory pressure. `network`: skipped while offline.
//!
//! A job reports on the last line of its stdout ([`Report`]); it notifies the user through its
//! app's **outbox** ([`notify`]): `<root>/<app>/outbox/<id>.json`, one file per notification,
//! drained (read, then removed) by the helper. Writing the same id again replaces a pending one.
use crate::apps::{AppSpec, Installed, Locations, Os};
use crate::paths::Root;
use chrono::{DateTime, Local, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SCHEMA: u32 = 1;
pub const MANIFEST: &str = "otherwise-background.json";
pub const MAX_MANIFEST: u64 = 64 << 10;
pub const MAX_JOBS: usize = 16;
pub const OUTBOX: &str = "outbox";
pub const MAX_NOTIFICATION: u64 = 16 << 10;
/// Notifications waiting longer than this are dropped unseen (the helper was off, the app was
/// gone): a week-old "your bill is due today" helps nobody.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 3600);
pub const MIN_EVERY: Duration = Duration::from_secs(60);
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
pub const MAX_TIMEOUT: Duration = Duration::from_secs(3600);
pub const DEFAULT_MEMORY_MB: u64 = 512;
pub const MAX_MEMORY_MB: u64 = 4096;
/// Environment of a job run by the helper.
pub const ENV_JOB: &str = "OTHERWISE_BACKGROUND_JOB";
/// The user's language (`es`, `en`, `pt`, `fr`) for status text and notifications.
pub const ENV_LANG: &str = "OTHERWISE_LANG";
/// Prefix-free marker of the report line.
pub const REPORT_KEY: &str = "otherwiseJob";

/// A label in the app's own words: the helper can't know other apps' catalogs.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Label {
    #[serde(default)]
    pub key: String,
    pub en: String,
    #[serde(flatten)]
    pub other: BTreeMap<String, String>,
}

impl Label {
    pub fn en(key: &str, en: &str) -> Label {
        Label { key: key.into(), en: en.into(), other: BTreeMap::new() }
    }

    pub fn with(mut self, lang: &str, text: &str) -> Label {
        self.other.insert(lang.into(), text.into());
        self
    }

    /// `lang` (`es`, `pt-BR` → `pt`), else English.
    pub fn get(&self, lang: &str) -> &str {
        let base = lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        if base == "en" {
            return &self.en;
        }
        self.other.get(&base).filter(|s| !s.is_empty()).map(String::as_str).unwrap_or(&self.en)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    Every(Duration),
    /// Once a day at this local time (or as soon as possible after it).
    Daily(NaiveTime),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub label: Label,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
    #[serde(default = "yes")]
    pub while_open: bool,
    #[serde(default)]
    pub heavy: bool,
    #[serde(default)]
    pub network: bool,
}

fn yes() -> bool {
    true
}

impl Job {
    pub fn every(id: &str, label: Label, every: &str, args: &[&str]) -> Job {
        Job {
            id: id.into(),
            label,
            every: Some(every.into()),
            daily: None,
            exe: None,
            args: args.iter().map(|s| s.to_string()).collect(),
            timeout_secs: None,
            memory_mb: None,
            while_open: true,
            heavy: false,
            network: false,
        }
    }

    /// `daily` wins over `every`; None when neither parses.
    pub fn schedule(&self) -> Option<Schedule> {
        if let Some(d) = &self.daily {
            return NaiveTime::parse_from_str(d.trim(), "%H:%M").ok().map(Schedule::Daily);
        }
        parse_duration(self.every.as_deref()?).map(|d| Schedule::Every(d.max(MIN_EVERY)))
    }

    pub fn timeout(&self) -> Duration {
        self.timeout_secs.map(Duration::from_secs).unwrap_or(DEFAULT_TIMEOUT).clamp(Duration::from_secs(5), MAX_TIMEOUT)
    }

    pub fn memory_mb(&self) -> u64 {
        self.memory_mb.unwrap_or(DEFAULT_MEMORY_MB).clamp(16, MAX_MEMORY_MB)
    }

    /// The next run after one that started at `last` (None: never ran → now).
    pub fn next_run<Tz: TimeZone>(&self, last: Option<DateTime<Utc>>, now: DateTime<Utc>, tz: &Tz) -> Option<DateTime<Utc>> {
        let s = self.schedule()?;
        let Some(last) = last else {
            return Some(match s {
                Schedule::Every(_) => now,
                Schedule::Daily(t) => daily_at(t, now, tz).filter(|at| *at <= now).unwrap_or_else(|| daily_at(t, now, tz).unwrap_or(now)),
            });
        };
        Some(match s {
            Schedule::Every(d) => last + chrono::Duration::from_std(d).ok()?,
            Schedule::Daily(t) => {
                let today = daily_at(t, now, tz)?;
                if last >= today { today + chrono::Duration::days(1) } else { today }
            }
        })
    }
}

/// Today's `t` in `tz`, as UTC.
fn daily_at<Tz: TimeZone>(t: NaiveTime, now: DateTime<Utc>, tz: &Tz) -> Option<DateTime<Utc>> {
    let day = now.with_timezone(tz).date_naive();
    tz.from_local_datetime(&day.and_time(t)).earliest().map(|d| d.with_timezone(&Utc))
}

/// `30s`, `15m`, `2h`, `1d`; a bare number is seconds.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (num, mult) = match s.char_indices().last()? {
        (i, 's') => (&s[..i], 1),
        (i, 'm') => (&s[..i], 60),
        (i, 'h') => (&s[..i], 3600),
        (i, 'd') => (&s[..i], 86400),
        _ => (s, 1),
    };
    let n: u64 = num.trim().parse().ok()?;
    (n > 0).then(|| Duration::from_secs(n.saturating_mul(mult)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub app: String,
    #[serde(default)]
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    Missing,
    TooLarge,
    Unreadable(String),
    NewerSchema(u32),
    /// The file names another app.
    WrongApp(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Missing => write!(f, "no {MANIFEST}"),
            ManifestError::TooLarge => write!(f, "{MANIFEST} is too large"),
            ManifestError::Unreadable(e) => write!(f, "{MANIFEST}: {e}"),
            ManifestError::NewerSchema(s) => write!(f, "{MANIFEST} has schema {s}, newer than {SCHEMA}"),
            ManifestError::WrongApp(a) => write!(f, "{MANIFEST} is for {a}"),
        }
    }
}

/// Valid job id: like an app id (`a-z`, `0-9`, `-`), at most 40 characters.
pub fn valid_job_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !id.starts_with('-')
}

/// A file name, not a path.
fn valid_exe(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0'])
}

impl Manifest {
    pub fn parse(s: &str, app: &str) -> Result<Manifest, ManifestError> {
        let mut m: Manifest = serde_json::from_str(s).map_err(|e| ManifestError::Unreadable(e.to_string()))?;
        if m.schema > SCHEMA {
            return Err(ManifestError::NewerSchema(m.schema));
        }
        if m.app != app {
            return Err(ManifestError::WrongApp(m.app));
        }
        let mut seen = std::collections::BTreeSet::new();
        m.jobs.retain(|j| valid_job_id(&j.id) && seen.insert(j.id.clone()) && j.schedule().is_some() && j.exe.as_deref().is_none_or(valid_exe) && !j.label.en.trim().is_empty());
        m.jobs.truncate(MAX_JOBS);
        Ok(m)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
}

/// Where an install keeps its manifest.
pub fn manifest_path(os: Os, install: &Path) -> PathBuf {
    match os {
        Os::Macos => install.join("Contents").join("Resources").join(MANIFEST),
        Os::Windows | Os::Linux => install.join(MANIFEST),
    }
}

/// Where an install keeps its executables.
pub fn exe_dir(os: Os, install: &Path) -> PathBuf {
    match os {
        Os::Macos => install.join("Contents").join("MacOS"),
        Os::Windows | Os::Linux => install.to_path_buf(),
    }
}

pub fn load(os: Os, install: &Path, app: &str) -> Result<Manifest, ManifestError> {
    let p = manifest_path(os, install);
    let meta = std::fs::metadata(&p).map_err(|_| ManifestError::Missing)?;
    if !meta.is_file() {
        return Err(ManifestError::Missing);
    }
    if meta.len() > MAX_MANIFEST {
        return Err(ManifestError::TooLarge);
    }
    let s = std::fs::read_to_string(&p).map_err(|e| ManifestError::Unreadable(e.to_string()))?;
    Manifest::parse(&s, app)
}

/// An installed app that declares background jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub spec: AppSpec,
    pub installed: Installed,
    /// The bundle / install folder (always set: CLI-only installs on PATH don't count).
    pub install: PathBuf,
    pub manifest: Manifest,
}

/// Every known app installed here (bundle / install folder, not PATH) that ships a manifest
/// with at least one job. Apps without one, or with an unreadable one, have no jobs.
pub fn scan(loc: &Locations) -> Vec<Registered> {
    scan_specs(&crate::apps::known_specs(), loc)
}

pub fn scan_specs(specs: &[AppSpec], loc: &Locations) -> Vec<Registered> {
    specs.iter().filter_map(|s| registered(s, loc)).collect()
}

pub fn registered(spec: &AppSpec, loc: &Locations) -> Option<Registered> {
    let installed = crate::apps::detect_install(spec, loc)?;
    let install = installed.path.clone()?;
    if loc.os == Os::Macos && installed.bundle_id.as_deref() != Some(spec.bundle_id.as_str()) {
        // a bundle without a readable identifier isn't provably ours
        return None;
    }
    let manifest = load(loc.os, &install, &spec.id).ok()?;
    (!manifest.jobs.is_empty()).then_some(Registered { spec: spec.clone(), installed, install, manifest })
}

/// Why a job can't run now. The helper skips quietly and stops scheduling the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotRunnable {
    /// The app is no longer installed where it was (deleted, moved, replaced by another app).
    NotInstalled,
    /// The install no longer declares this job.
    NoSuchJob,
    /// The executable is missing, not a file, not executable, or resolves outside the install.
    BadExecutable(PathBuf),
}

/// What to start: the verified program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub job: Job,
}

/// Re-checks, right before a run, that `reg`'s app is still installed at the same place with the
/// same identity and still declares `job`, and that the job's executable is a real executable
/// file inside the install (symlinks resolved).
pub fn plan(reg: &Registered, job: &str, loc: &Locations) -> Result<Plan, NotRunnable> {
    let now = registered(&reg.spec, loc).ok_or(NotRunnable::NotInstalled)?;
    if now.install != reg.install {
        return Err(NotRunnable::NotInstalled);
    }
    let job = now.manifest.jobs.iter().find(|j| j.id == job).cloned().ok_or(NotRunnable::NoSuchJob)?;
    let dir = exe_dir(loc.os, &now.install);
    let name = job.exe.clone().unwrap_or_else(|| reg.spec.binary.clone());
    let file = if loc.os == Os::Windows && !name.to_ascii_lowercase().ends_with(".exe") { format!("{name}.exe") } else { name };
    let program = dir.join(&file);
    let bad = || NotRunnable::BadExecutable(program.clone());
    let real = std::fs::canonicalize(&program).map_err(|_| bad())?;
    let real_dir = std::fs::canonicalize(&dir).map_err(|_| bad())?;
    if !real.starts_with(&real_dir) || !real.is_file() || !is_executable(&real, loc.os) {
        return Err(bad());
    }
    Ok(Plan { program, args: job.args.clone(), job })
}

/// By the install's layout: a Windows install runs `.exe` files; elsewhere the execute bits
/// (where the host has them: a macOS / Linux layout read on Windows, in tests, has none).
fn is_executable(p: &Path, os: Os) -> bool {
    if os == Os::Windows {
        return p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

/// What a job says about its run, as the last line of its stdout:
/// `{"otherwiseJob": 1, "status": "2 new receipts", "idle": false}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// One short line for the helper's menu, in the user's language ([`ENV_LANG`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Nothing is configured (no mail account, no devices): not an error, nothing to show.
    #[serde(default)]
    pub idle: bool,
}

impl Report {
    pub fn status(s: impl Into<String>) -> Report {
        Report { status: Some(s.into()), idle: false }
    }

    pub fn idle() -> Report {
        Report { status: None, idle: true }
    }

    /// The line to print.
    pub fn line(&self) -> String {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.insert(REPORT_KEY.into(), SCHEMA.into());
        }
        v.to_string()
    }

    /// The last report line of a job's output, if any.
    pub fn from_output(stdout: &str) -> Option<Report> {
        stdout.lines().rev().map(str::trim).filter(|l| l.starts_with('{')).find_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            v.get(REPORT_KEY)?;
            let mut r: Report = serde_json::from_value(v).ok()?;
            r.status = r.status.map(|s| crate::text::truncate_chars(s.trim(), 120)).filter(|s| !s.is_empty());
            Some(r)
        })
    }
}

/// Is this process a job run by the helper (and which)?
pub fn current_job() -> Option<String> {
    std::env::var(ENV_JOB).ok().filter(|s| valid_job_id(s))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub schema: u32,
    /// Stable per app: the same reminder keeps its id, so it is shown once.
    pub id: String,
    /// Filled in by the reader from the folder; writers may leave it out.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// `otherwise://…`, opened when the notification is clicked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default)]
    pub severity: crate::today::Severity,
    /// RFC 3339.
    pub created_at: String,
}

impl Notification {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Notification {
        Notification { schema: SCHEMA, id: id.into(), app: String::new(), title: title.into(), body: None, link: None, severity: Default::default(), created_at: Utc::now().to_rfc3339() }
    }

    pub fn body(mut self, b: impl Into<String>) -> Notification {
        self.body = Some(b.into());
        self
    }

    pub fn link(mut self, l: &crate::Link) -> Notification {
        self.link = Some(l.to_url());
        self
    }

    pub fn severity(mut self, s: crate::today::Severity) -> Notification {
        self.severity = s;
        self
    }

    /// `<app>/<id>`, the key a helper remembers it showed.
    pub fn key(&self) -> String {
        format!("{}/{}", self.app, self.id)
    }

    fn created(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.created_at).ok().map(|d| d.with_timezone(&Utc))
    }
}

pub fn outbox_dir(root: &Root, app: &str) -> PathBuf {
    root.app_dir(app).join(OUTBOX)
}

/// Ids become file names: keep `[a-z0-9._-]`, the rest as `_`, at most 100 characters.
fn file_stem(id: &str) -> String {
    let s: String = id.chars().take(100).map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c.to_ascii_lowercase() } else { '_' }).collect();
    let s = s.trim_start_matches('.').to_string();
    if s.is_empty() { "_".into() } else { s }
}

/// Queues a notification for the helper (an app writes only its own outbox).
pub fn notify(root: &Root, app: &str, n: &Notification) -> std::io::Result<()> {
    let mut n = n.clone();
    n.schema = SCHEMA;
    n.app = String::new();
    n.title = crate::text::truncate_chars(n.title.trim(), 120);
    n.body = n.body.map(|b| crate::text::truncate_chars(b.trim(), 400));
    let path = outbox_dir(root, app).join(format!("{}.json", file_stem(&n.id)));
    crate::fsutil::write_atomic(&path, (serde_json::to_string_pretty(&n).unwrap_or_default() + "\n").as_bytes())
}

/// Takes every pending notification of `app` out of its outbox: unreadable files and ones older
/// than [`MAX_AGE`] are removed without being returned. Oldest first.
pub fn drain(root: &Root, app: &str, now: DateTime<Utc>) -> Vec<Notification> {
    let dir = outbox_dir(root, app);
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out = vec![];
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let path = e.path();
        let n = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.is_file() && m.len() <= MAX_NOTIFICATION)
            .and_then(|_| std::fs::read_to_string(&path).ok())
            .and_then(|s| serde_json::from_str::<Notification>(&s).ok());
        if std::fs::remove_file(&path).is_err() {
            continue; // someone else drained it (another computer's helper on a synced root)
        }
        let Some(mut n) = n else { continue };
        let fresh = n.created().is_some_and(|c| now.signed_duration_since(c).to_std().map(|a| a <= MAX_AGE).unwrap_or(true));
        if fresh && n.schema <= SCHEMA && !n.title.is_empty() {
            n.app = app.to_string();
            out.push(n);
        }
    }
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    out
}

/// Has anything been queued for `app`? (Cheap: a directory listing.)
pub fn pending(root: &Root, app: &str) -> bool {
    std::fs::read_dir(outbox_dir(root, app)).is_ok_and(|mut rd| rd.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".json"))))
}

/// The local wall clock, for callers that don't inject one.
pub fn local_next_run(job: &Job, last: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    job.next_run(last, Utc::now(), &Local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("15m"), Some(Duration::from_secs(900)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("1d"), Some(Duration::from_secs(86400)));
        assert_eq!(parse_duration("45"), Some(Duration::from_secs(45)));
        assert_eq!(parse_duration("0m"), None);
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn labels() {
        let l = Label::en("k", "Check mail").with("es", "Revisar correo");
        assert_eq!(l.get("es-CO"), "Revisar correo");
        assert_eq!(l.get("fr"), "Check mail");
        assert_eq!(l.get("en"), "Check mail");
        let j: Label = serde_json::from_str(r#"{"key":"k","en":"A","pt":"B"}"#).unwrap();
        assert_eq!(j.get("pt_BR"), "B");
    }

    #[test]
    fn schedules() {
        let tz = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-11-23T15:00:00Z").unwrap().with_timezone(&Utc); // 10:00 local
        let mut j = Job::every("mail", Label::en("k", "Mail"), "15m", &[]);
        assert_eq!(j.next_run(None, now, &tz), Some(now));
        assert_eq!(j.next_run(Some(now), now, &tz), Some(now + chrono::Duration::minutes(15)));
        j.every = Some("5s".into()); // clamped to a minute
        assert_eq!(j.next_run(Some(now), now, &tz), Some(now + chrono::Duration::minutes(1)));
        j.daily = Some("08:00".into());
        let at8 = DateTime::parse_from_rfc3339("2026-11-23T13:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(j.next_run(None, now, &tz), Some(at8), "past 08:00 today and never ran: due now");
        assert_eq!(j.next_run(Some(at8), now, &tz), Some(at8 + chrono::Duration::days(1)));
        let yesterday = at8 - chrono::Duration::hours(20);
        assert_eq!(j.next_run(Some(yesterday), now, &tz), Some(at8));
        j.daily = Some("18:30".into());
        let at1830 = DateTime::parse_from_rfc3339("2026-11-23T23:30:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(j.next_run(None, now, &tz), Some(at1830));
        j.daily = Some("25:00".into());
        assert_eq!(j.schedule(), None);
    }

    #[test]
    fn manifest_validation() {
        let raw = r#"{"schema":1,"app":"expense","jobs":[
            {"id":"mail","label":{"en":"Mail"},"every":"15m","args":["background","run","mail"],"heavy":true},
            {"id":"mail","label":{"en":"Dup"},"every":"15m"},
            {"id":"Bad Id","label":{"en":"x"},"every":"15m"},
            {"id":"noschedule","label":{"en":"x"}},
            {"id":"escape","label":{"en":"x"},"every":"1h","exe":"../../../bin/sh"},
            {"id":"nolabel","label":{"en":" "},"every":"1h"},
            {"id":"bills","label":{"en":"Bills"},"daily":"08:00","exe":"expense-cli","future":1}]}"#;
        let m = Manifest::parse(raw, "expense").unwrap();
        let ids: Vec<&str> = m.jobs.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, ["mail", "bills"]);
        assert!(m.jobs[0].heavy && m.jobs[0].while_open && !m.jobs[0].network);
        assert_eq!(Manifest::parse(raw, "vigia"), Err(ManifestError::WrongApp("expense".into())));
        assert_eq!(Manifest::parse(r#"{"schema":2,"app":"expense"}"#, "expense"), Err(ManifestError::NewerSchema(2)));
        assert!(matches!(Manifest::parse("{", "expense"), Err(ManifestError::Unreadable(_))));
        let back = Manifest::parse(&m.to_json(), "expense").unwrap();
        assert_eq!(back, m);
        assert_eq!(m.jobs[0].timeout(), DEFAULT_TIMEOUT);
        assert_eq!(m.jobs[0].memory_mb(), DEFAULT_MEMORY_MB);
    }

    #[test]
    fn reports() {
        let out = format!("progress…\n{}\n\n", Report::status("2 new receipts").line());
        assert_eq!(Report::from_output(&out), Some(Report::status("2 new receipts")));
        assert_eq!(Report::from_output(&Report::idle().line()), Some(Report::idle()));
        assert_eq!(Report::from_output("{\"status\":\"not a report\"}"), None);
        assert_eq!(Report::from_output("nothing"), None);
    }

    #[test]
    fn stems() {
        assert_eq!(file_stem("bill-42/due"), "bill-42_due");
        assert_eq!(file_stem("../x"), "_x");
        assert_eq!(file_stem(""), "_");
    }
}
