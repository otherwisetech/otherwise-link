//! Opening links and URLs with the system's handler.
//!
//! [`open_link`] tries the target app's own scheme first (`otherwise-vigia://…`) and falls back
//! to `otherwise://…`, which Otherwise Apps handles (it can say the app isn't installed and offer
//! to install it). The launcher itself must use [`open_app_link`] when forwarding, or a link to
//! a missing app would bounce back to it forever.
use crate::link::{self, Link};
use std::io;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// The app's own scheme handled it.
    App,
    /// It went to `otherwise://` (the launcher).
    Hub,
}

/// The system "open this URL" command for the current OS (the URL is one argument, never
/// through a shell).
pub fn opener(url: &str) -> Command {
    let mut c;
    if cfg!(target_os = "macos") {
        c = Command::new("open");
        c.arg(url);
    } else if cfg!(windows) {
        c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler").arg(url);
    } else {
        c = Command::new("xdg-open");
        c.arg(url);
    }
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c
}

/// Windows: whether `scheme` has a handler (`rundll32` shows a "pick an app" dialog instead of
/// failing). Elsewhere: None, the opener's exit status tells.
pub fn scheme_registered(scheme: &str) -> Option<bool> {
    if !cfg!(windows) {
        return None;
    }
    let st = Command::new("reg").args(["query", &format!("HKCR\\{scheme}"), "/v", "URL Protocol"]).stdout(Stdio::null()).stderr(Stdio::null()).status().ok()?;
    Some(st.success())
}

pub fn open_url(url: &str) -> io::Result<()> {
    if let Some((scheme, _)) = url.split_once("://")
        && scheme_registered(scheme) == Some(false)
    {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("no handler for {scheme}://")));
    }
    let st = opener(url).status()?;
    if st.success() { Ok(()) } else { Err(io::Error::other(format!("no handler for {url} ({st})"))) }
}

/// Only the app's own scheme; no fallback.
pub fn open_app_link(l: &Link) -> io::Result<()> {
    open_url(&l.app_url())
}

pub fn open_link(l: &Link) -> io::Result<Opened> {
    if l.app != link::HUB_APP && open_app_link(l).is_ok() {
        return Ok(Opened::App);
    }
    open_url(&l.to_url()).map(|_| Opened::Hub)
}
