//! Deep links: `otherwise://<app>/<path>?<query>`.
//!
//! - `otherwise://vigia/item/42` opens item 42 in Vigía;
//!   `otherwise://nodo/device/6f0c…?map=home` a device on a map;
//!   `otherwise://otherwise-apps/today` the launcher's Today view.
//! - Each app registers its own scheme, `otherwise-<app>` (`otherwise-vigia://item/42`), and the
//!   launcher (Otherwise Hub) registers `otherwise://` and forwards to the app. Both forms parse
//!   to the same [`Link`]; [`Link::to_url`] writes the canonical `otherwise://` form and
//!   [`Link::app_url`] the per-app one.
//!
//! Path segments and query values are percent-encoded (RFC 3986 unreserved characters stay as
//! they are); `+` in a query reads as a space. Fragments are dropped. App ids are
//! `[a-z0-9][a-z0-9-]*`, at most 64 characters.
use std::fmt;

pub const SCHEME: &str = "otherwise";
/// Per-app schemes are this prefix plus the app id.
pub const APP_SCHEME_PREFIX: &str = "otherwise-";
/// The launcher (Otherwise Hub) owns `otherwise://` itself, so its links have no per-app form.
pub const HUB_APP: &str = "otherwise-apps";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Link {
    pub app: String,
    /// Decoded path segments (`["item", "42"]`).
    pub path: Vec<String>,
    /// Decoded query pairs, in order; keys may repeat.
    pub query: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    NotALink,
    Scheme(String),
    App(String),
    Encoding,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::NotALink => write!(f, "not an otherwise:// link"),
            LinkError::Scheme(s) => write!(f, "unknown scheme {s:?}"),
            LinkError::App(a) => write!(f, "invalid app id {a:?}"),
            LinkError::Encoding => write!(f, "bad percent-encoding"),
        }
    }
}

impl std::error::Error for LinkError {}

/// `[a-z0-9][a-z0-9-]*`, 1–64 characters.
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !id.starts_with('-')
}

/// The scheme an app registers for itself: `otherwise-vigia`.
pub fn app_scheme(app: &str) -> String {
    format!("{APP_SCHEME_PREFIX}{app}")
}

impl Link {
    /// Panics on an invalid app id: ids are compile-time constants in practice.
    pub fn new(app: &str) -> Link {
        assert!(valid_app_id(app), "invalid app id {app:?}");
        Link { app: app.to_string(), ..Default::default() }
    }

    /// Appends path segments; each is encoded on its own, so `/` inside one is kept.
    pub fn segment(mut self, s: impl Into<String>) -> Link {
        self.path.push(s.into());
        self
    }

    pub fn param(mut self, k: impl Into<String>, v: impl Into<String>) -> Link {
        self.query.push((k.into(), v.into()));
        self
    }

    /// The first value of query key `k`.
    pub fn get(&self, k: &str) -> Option<&str> {
        self.query.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
    }

    /// Segment `i`, if there.
    pub fn at(&self, i: usize) -> Option<&str> {
        self.path.get(i).map(String::as_str)
    }

    pub fn parse(s: &str) -> Result<Link, LinkError> {
        let s = s.trim();
        let (scheme, rest) = s.split_once("://").ok_or(LinkError::NotALink)?;
        let scheme = scheme.to_ascii_lowercase();
        let rest = rest.split('#').next().unwrap_or("");
        let (path_part, query_part) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        let mut segs = path_part.split('/').filter(|p| !p.is_empty());
        let app = if scheme == SCHEME {
            segs.next().ok_or(LinkError::App(String::new()))?.to_ascii_lowercase()
        } else if let Some(app) = scheme.strip_prefix(APP_SCHEME_PREFIX) {
            app.to_string()
        } else {
            return Err(LinkError::Scheme(scheme));
        };
        if !valid_app_id(&app) {
            return Err(LinkError::App(app));
        }
        let path = segs.map(|p| decode(p, false)).collect::<Result<Vec<_>, _>>()?;
        let mut query = Vec::new();
        for pair in query_part.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            query.push((decode(k, true)?, decode(v, true)?));
        }
        Ok(Link { app, path, query })
    }

    fn tail(&self) -> String {
        let mut out = String::new();
        for (i, p) in self.path.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(&encode(p));
        }
        if !self.query.is_empty() {
            out.push('?');
            let pairs: Vec<String> = self.query.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect();
            out.push_str(&pairs.join("&"));
        }
        out
    }

    /// `otherwise://vigia/item/42?x=1`
    pub fn to_url(&self) -> String {
        let tail = self.tail();
        if tail.is_empty() {
            format!("{SCHEME}://{}", self.app)
        } else if tail.starts_with('?') {
            format!("{SCHEME}://{}{tail}", self.app)
        } else {
            format!("{SCHEME}://{}/{tail}", self.app)
        }
    }

    /// `otherwise-vigia://item/42?x=1`: what the app itself registered.
    pub fn app_url(&self) -> String {
        if self.app == HUB_APP {
            return self.to_url();
        }
        format!("{}://{}", app_scheme(&self.app), self.tail())
    }

    /// The first argument that is a link: Linux and Windows hand the URL to the app as an
    /// argument (`Exec=app %u`, `"app.exe" "%1"`); macOS sends it as an Apple event instead (GPUI:
    /// `Application::on_open_urls`).
    pub fn from_args<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) -> Option<Link> {
        args.into_iter().find_map(|a| Link::parse(a.as_ref()).ok())
    }
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_url())
    }
}

impl std::str::FromStr for Link {
    type Err = LinkError;
    fn from_str(s: &str) -> Result<Link, LinkError> {
        Link::parse(s)
    }
}

pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn decode(s: &str, plus_is_space: bool) -> Result<String, LinkError> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3).ok_or(LinkError::Encoding)?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| LinkError::Encoding)?);
                i += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| LinkError::Encoding)
}
