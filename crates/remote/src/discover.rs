//! "In your tailnet": the computers the local Tailscale knows (`tailscale status --json`), and
//! which of them answer `/v1/hello` as an Otherwise server. Nothing here needs Tailscale to be
//! installed: no CLI, no peers.
use crate::client::Client;
use crate::types::Hello;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub host_name: String,
    /// MagicDNS name without the trailing dot (`studio.tail4cc17.ts.net`).
    pub dns_name: String,
    pub os: String,
    pub online: bool,
}

impl Peer {
    pub fn url(&self) -> String {
        format!("https://{}", self.dns_name)
    }
}

/// The Tailscale CLI: `$TAILSCALE_CLI`, `tailscale` on `PATH`, or the one inside the macOS app
/// (the App Store / standalone variant has none on `PATH`).
pub fn tailscale_cli() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TAILSCALE_CLI").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let exe = if cfg!(windows) { "tailscale.exe" } else { "tailscale" };
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            let p = d.join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    [
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
        "/usr/bin/tailscale",
        r"C:\Program Files\Tailscale\tailscale.exe",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// The peers in `tailscale status --json` (not this computer), online first.
pub fn parse_status(json: &str) -> Vec<Peer> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut out: Vec<Peer> = v
        .get("Peer")
        .and_then(|p| p.as_object())
        .map(|m| {
            m.values()
                .filter_map(|p| {
                    let dns = p.get("DNSName")?.as_str()?.trim_end_matches('.').to_string();
                    (!dns.is_empty()).then(|| Peer { host_name: p.get("HostName").and_then(|h| h.as_str()).unwrap_or("").to_string(), dns_name: dns, os: p.get("OS").and_then(|h| h.as_str()).unwrap_or("").to_string(), online: p.get("Online").and_then(|o| o.as_bool()).unwrap_or(false) })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.dns_name.cmp(&b.dns_name)));
    out
}

/// This computer's peers, from the local Tailscale (empty when it isn't installed or running).
pub fn peers() -> Vec<Peer> {
    let Some(cli) = tailscale_cli() else { return Vec::new() };
    let out = run_with_timeout(std::process::Command::new(cli).args(["status", "--json"]).env("TAILSCALE_BE_CLI", "1"), Duration::from_secs(4));
    out.map(|s| parse_status(&s)).unwrap_or_default()
}

fn run_with_timeout(cmd: &mut std::process::Command, timeout: Duration) -> Option<String> {
    let mut child = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut s).ok().map(|_| s)
    });
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    reader.join().ok().flatten()
}

/// Ask every online peer's `/v1/hello` at once (each within `timeout`); the ones that answer
/// as an Otherwise server, in the peers' order.
pub fn probe(peers: &[Peer], timeout: Duration) -> Vec<(Peer, Hello)> {
    probe_with(peers, timeout, |p| p.url())
}

/// [`probe`] with the URL of each peer chosen by `url` (tests point it at a local server).
pub fn probe_with(peers: &[Peer], timeout: Duration, url: impl Fn(&Peer) -> String) -> Vec<(Peer, Hello)> {
    let handles: Vec<_> = peers
        .iter()
        .filter(|p| p.online)
        .take(64)
        .map(|p| {
            let (p, u) = (p.clone(), url(p));
            std::thread::spawn(move || Client::with_timeout(&u, timeout).hello().ok().map(|h| (p, h)))
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peers_from_status_json() {
        let json = r#"{"Self": {"DNSName": "macbookpro.t.ts.net."},
            "Peer": {"a": {"HostName": "studio", "DNSName": "studio.t.ts.net.", "OS": "macOS", "Online": true},
                     "b": {"HostName": "old", "DNSName": "old.t.ts.net.", "OS": "linux", "Online": false},
                     "c": {"HostName": "nodns", "DNSName": ""}}}"#;
        let p = parse_status(json);
        assert_eq!(p.iter().map(|p| p.dns_name.as_str()).collect::<Vec<_>>(), ["studio.t.ts.net", "old.t.ts.net"]);
        assert_eq!(p[0].url(), "https://studio.t.ts.net");
        assert!(parse_status("not json").is_empty());
    }
}
