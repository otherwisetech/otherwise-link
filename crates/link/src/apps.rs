//! The apps we know about and where they are installed.
//!
//! [`KNOWN`] lists every Otherwise app with what it can do; [`detect`] / [`discover`] find
//! installed copies:
//!
//! - macOS: `<Name>.app` in `/Applications` and `~/Applications`; the bundle's
//!   `CFBundleIdentifier` must match (a same-named app from someone else is not ours), the
//!   version comes from `CFBundleShortVersionString`, the command line from `Contents/MacOS`.
//! - Windows: `%LOCALAPPDATA%\Programs\Otherwise\<install name>\` (where Otherwise Hub installs),
//!   then `%ProgramFiles%\Otherwise\<install name>\`.
//! - Linux: `~/.local/opt/otherwise/<install name>/`, then `/opt/otherwise/<install name>/`.
//! - Everywhere: the command-line binary on `PATH` (plus the usual bin folders GUI apps don't
//!   get in their `PATH`), so a CLI-only install counts too.
//!
//! On Windows and Linux the version comes from the `.otherwise-install.json` marker Otherwise
//! Apps writes next to the binary. Every location is a field of [`Locations`], so tests point it
//! at temp dirs.
use crate::link;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    /// Registers `otherwise-<id>://` and opens [`crate::Link`]s.
    DeepLinks,
    /// Publishes a Today feed.
    Today,
    /// Reads / writes the shared entity registry.
    Entities,
    /// `<cli> mcp` is a stdio MCP server (proxied by `otherwise-hub`).
    Mcp,
    /// Publishes an events feed (`<root>/<app>/events.json`) for the calendar.
    Calendar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Macos,
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// One app, as data. [`KNOWN`] holds the built-in ones; apps not listed there (a newer catalog
/// entry) can be described at runtime with the same struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSpec {
    /// Stable id: deep-link host, Today file name, `refs` key, MCP tool prefix.
    pub id: String,
    pub name: String,
    /// macOS `CFBundleIdentifier`; also the Linux `.desktop` file name.
    pub bundle_id: String,
    /// macOS bundle name without `.app`.
    pub app_name: String,
    /// Folder on Windows / Linux.
    pub install_name: String,
    /// The GUI executable (inside `Contents/MacOS` or the install folder), without `.exe`.
    pub binary: String,
    /// Command-line binaries, in order of preference (inside the bundle / folder, else on PATH).
    pub cli: Vec<String>,
    /// Arguments that start its MCP server, and the extra one that enables its write tools.
    pub mcp_args: Vec<String>,
    pub mcp_write_arg: Option<String>,
    pub open_source: bool,
    pub capabilities: Vec<Capability>,
}

impl AppSpec {
    pub fn has(&self, c: Capability) -> bool {
        self.capabilities.contains(&c)
    }

    /// `otherwise-<id>` when the app takes deep links.
    pub fn scheme(&self) -> Option<String> {
        self.has(Capability::DeepLinks).then(|| link::app_scheme(&self.id))
    }
}

/// Static form of [`AppSpec`] for the built-in table.
#[derive(Debug, Clone, Copy)]
pub struct KnownApp {
    pub id: &'static str,
    pub name: &'static str,
    pub bundle_id: &'static str,
    pub app_name: &'static str,
    pub install_name: &'static str,
    pub binary: &'static str,
    pub cli: &'static [&'static str],
    pub mcp_args: &'static [&'static str],
    pub mcp_write_arg: Option<&'static str>,
    pub open_source: bool,
    pub capabilities: &'static [Capability],
}

impl KnownApp {
    pub fn spec(&self) -> AppSpec {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
        AppSpec {
            id: self.id.into(),
            name: self.name.into(),
            bundle_id: self.bundle_id.into(),
            app_name: self.app_name.into(),
            install_name: self.install_name.into(),
            binary: self.binary.into(),
            cli: s(self.cli),
            mcp_args: s(self.mcp_args),
            mcp_write_arg: self.mcp_write_arg.map(str::to_string),
            open_source: self.open_source,
            capabilities: self.capabilities.to_vec(),
        }
    }
}

use Capability::*;

/// The launcher itself; it registers `otherwise://` and forwards links.
pub const HUB_ID: &str = link::HUB_APP;

/// Capabilities list what the integration contract allows for each app; an app that hasn't
/// shipped one yet simply isn't found doing it (no `mcp` subcommand → the hub skips it).
pub static KNOWN: &[KnownApp] = &[
    KnownApp { id: HUB_ID, name: "Otherwise Hub", bundle_id: "io.github.otherwisetech.otherwise-apps", app_name: "Otherwise Apps", install_name: "otherwise-apps", binary: "otherwise-apps", cli: &[], mcp_args: &[], mcp_write_arg: None, open_source: true, capabilities: &[DeepLinks, Today, Entities] },
    KnownApp { id: "expense", name: "expense Business", bundle_id: "io.github.otherwisetech.expense", app_name: "expense", install_name: "expense", binary: "expense", cli: &["expense-cli", "expense"], mcp_args: &["mcp"], mcp_write_arg: Some("--allow-writes"), open_source: false, capabilities: &[DeepLinks, Today, Entities, Mcp, Calendar] },
    KnownApp { id: "vigia", name: "Vigía", bundle_id: "io.github.otherwisetech.vigia-app", app_name: "Vigía", install_name: "vigia", binary: "vigia-app", cli: &["vigia"], mcp_args: &["mcp"], mcp_write_arg: Some("--allow-writes"), open_source: false, capabilities: &[DeepLinks, Today, Entities, Mcp, Calendar] },
    KnownApp { id: "nodo", name: "Nodo", bundle_id: "io.github.otherwisetech.nodo-app", app_name: "Nodo", install_name: "nodo", binary: "nodo-app", cli: &["nodo"], mcp_args: &["mcp"], mcp_write_arg: Some("--allow-writes"), open_source: false, capabilities: &[DeepLinks, Today, Entities, Mcp] },
    KnownApp { id: "genealogy", name: "genealogy", bundle_id: "io.github.otherwisetech.genealogy-rs", app_name: "genealogy-rs", install_name: "genealogy-rs", binary: "genealogy-rs", cli: &[], mcp_args: &[], mcp_write_arg: None, open_source: true, capabilities: &[DeepLinks, Entities, Calendar] },
    KnownApp { id: "mosaico", name: "mosaico", bundle_id: "io.github.otherwisetech.mosaico", app_name: "mosaico", install_name: "mosaico", binary: "mosaico", cli: &[], mcp_args: &[], mcp_write_arg: None, open_source: true, capabilities: &[DeepLinks, Today] },
    KnownApp { id: "diskmap", name: "diskmap", bundle_id: "io.github.otherwisetech.diskmap", app_name: "diskmap", install_name: "diskmap", binary: "diskmap", cli: &[], mcp_args: &[], mcp_write_arg: None, open_source: true, capabilities: &[DeepLinks, Today] },
    KnownApp { id: "rama", name: "rama", bundle_id: "io.github.otherwisetech.rama", app_name: "rama", install_name: "rama", binary: "rama", cli: &[], mcp_args: &[], mcp_write_arg: None, open_source: true, capabilities: &[DeepLinks, Today] },
    KnownApp { id: "brujula", name: "Brújula", bundle_id: "com.otherwisetech.brujula", app_name: "Brújula", install_name: "brujula", binary: "brujula", cli: &[], mcp_args: &[], mcp_write_arg: None, open_source: false, capabilities: &[DeepLinks, Calendar] },
    KnownApp { id: "helio", name: "Helio", bundle_id: "com.otherwisetech.helio", app_name: "Helio", install_name: "helio", binary: "helio", cli: &[], mcp_args: &[], mcp_write_arg: None, open_source: false, capabilities: &[DeepLinks, Calendar] },
    KnownApp { id: "mi-huerta", name: "Mi Huerta", bundle_id: "com.otherwisetech.mi-huerta", app_name: "Mi Huerta", install_name: "mi-huerta", binary: "mi-huerta", cli: &["mi-huerta-cli"], mcp_args: &["mcp"], mcp_write_arg: None, open_source: true, capabilities: &[DeepLinks, Mcp, Calendar] },
    // one binary: the GUI also answers `mcp [--book <folder>] [--propose | --allow-writes]`
    KnownApp { id: "expense-me", name: "expense me", bundle_id: "com.otherwisetech.expense-me", app_name: "expense me", install_name: "expense-me", binary: "expense-me", cli: &["expense-me"], mcp_args: &["mcp"], mcp_write_arg: Some("--allow-writes"), open_source: false, capabilities: &[DeepLinks, Mcp, Calendar] },
    // the Surco suite's launcher; its tools (com.otherwisetech.surco.<tool>) describe themselves
    // with an `otherwise-app.json` (APP_MANIFEST)
    KnownApp { id: "surco", name: "Surco", bundle_id: "com.otherwisetech.surco", app_name: "Surco", install_name: "surco", binary: "surco", cli: &[], mcp_args: &[], mcp_write_arg: None, open_source: false, capabilities: &[] },
];

pub fn known(id: &str) -> Option<&'static KnownApp> {
    KNOWN.iter().find(|a| a.id == id)
}

pub fn known_specs() -> Vec<AppSpec> {
    KNOWN.iter().map(KnownApp::spec).collect()
}

/// Display name of an app id: the known name, else the id.
pub fn display_name(id: &str) -> String {
    known(id).map(|a| a.name.to_string()).unwrap_or_else(|| id.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    pub os: Os,
    /// macOS: folders holding `.app` bundles, in lookup order.
    pub mac_dirs: Vec<PathBuf>,
    /// Windows / Linux: folders holding one folder per app, in lookup order.
    pub roots: Vec<PathBuf>,
    /// Where command-line binaries are looked for.
    pub path: Vec<PathBuf>,
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl Locations {
    /// This machine's folders and `PATH`.
    pub fn system() -> Locations {
        let mut path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let home = crate::paths::home();
        // GUI apps start with a bare PATH (launchd), so add where CLIs usually get installed
        for extra in [home.join(".local").join("bin"), home.join(".cargo").join("bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")] {
            if !path.contains(&extra) {
                path.push(extra);
            }
        }
        Locations::for_os(Os::current(), &home, path)
    }

    pub fn for_os(os: Os, home: &Path, path: Vec<PathBuf>) -> Locations {
        match os {
            Os::Macos => Locations { os, mac_dirs: vec![PathBuf::from("/Applications"), home.join("Applications")], roots: vec![], path },
            Os::Windows => {
                let local = env_dir("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local"));
                let mut roots = vec![local.join("Programs").join("Otherwise")];
                if let Some(pf) = env_dir("ProgramFiles") {
                    roots.push(pf.join("Otherwise"));
                }
                Locations { os, mac_dirs: vec![], roots, path }
            }
            Os::Linux => Locations { os, mac_dirs: vec![], roots: vec![home.join(".local").join("opt").join("otherwise"), PathBuf::from("/opt/otherwise")], path },
        }
    }

    fn exe_name(&self, base: &str) -> String {
        if self.os == Os::Windows { format!("{base}.exe") } else { base.to_string() }
    }

    /// `name` in the first [`Locations::path`] folder that has it.
    pub fn which(&self, name: &str) -> Option<PathBuf> {
        let file = self.exe_name(name);
        self.path.iter().map(|d| d.join(&file)).find(|p| p.is_file())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub id: String,
    /// The `.app` bundle or install folder; None for a CLI found only on PATH.
    pub path: Option<PathBuf>,
    /// The GUI executable.
    pub executable: Option<PathBuf>,
    /// The command line (next to the GUI first, else on PATH).
    pub cli: Option<PathBuf>,
    pub version: Option<String>,
    pub bundle_id: Option<String>,
}

/// The marker Otherwise Hub writes into Windows / Linux installs.
pub const MARKER: &str = ".otherwise-install.json";

/// `version` from the marker in `dir`, if it is `id`'s.
pub fn marker_version(dir: &Path, id: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(MARKER)).ok()?).ok()?;
    if v.get("id")?.as_str()? != id {
        return None;
    }
    let ver = v.get("version")?.as_str()?;
    Some(ver.strip_prefix('v').unwrap_or(ver).to_string())
}

/// The `<string>` value of `key` in an XML property list's top-level dict.
pub fn plist_string(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let rest = &xml[xml.find(&needle)? + needle.len()..];
    let body = rest.trim_start().strip_prefix("<string>")?;
    let end = body.find("</string>")?;
    Some(body[..end].trim().replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&"))
}

/// `Contents/Info.plist` as XML; binary plists go through `plutil` (macOS only).
pub fn read_info_plist(app: &Path) -> Option<String> {
    let path = app.join("Contents").join("Info.plist");
    let bytes = std::fs::read(&path).ok()?;
    if bytes.starts_with(b"bplist") {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let out = std::process::Command::new("plutil").args(["-convert", "xml1", "-o", "-"]).arg(&path).output().ok()?;
        return out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned());
    }
    String::from_utf8(bytes).ok()
}

fn find_cli(spec: &AppSpec, dir: Option<&Path>, loc: &Locations) -> Option<PathBuf> {
    if let Some(dir) = dir {
        for c in &spec.cli {
            let p = dir.join(loc.exe_name(c));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    spec.cli.iter().find_map(|c| loc.which(c))
}

/// Only the bundle / folder install (no PATH fallback).
pub fn detect_install(spec: &AppSpec, loc: &Locations) -> Option<Installed> {
    match loc.os {
        Os::Macos => {
            for dir in &loc.mac_dirs {
                let path = dir.join(format!("{}.app", spec.app_name));
                if !path.join("Contents").is_dir() {
                    continue;
                }
                let plist = read_info_plist(&path);
                let bundle_id = plist.as_deref().and_then(|p| plist_string(p, "CFBundleIdentifier"));
                if bundle_id.as_deref().is_some_and(|b| b != spec.bundle_id) {
                    continue;
                }
                let version = plist.as_deref().and_then(|p| plist_string(p, "CFBundleShortVersionString"));
                let macos = path.join("Contents").join("MacOS");
                let exe = macos.join(&spec.binary);
                let cli = find_cli(spec, Some(&macos), loc);
                return Some(Installed { id: spec.id.clone(), executable: exe.is_file().then_some(exe), cli, version, bundle_id, path: Some(path) });
            }
            None
        }
        Os::Windows | Os::Linux => {
            for root in &loc.roots {
                let path = root.join(&spec.install_name);
                let exe = path.join(loc.exe_name(&spec.binary));
                if !exe.is_file() {
                    continue;
                }
                let version = marker_version(&path, &spec.id);
                let cli = find_cli(spec, Some(&path), loc);
                return Some(Installed { id: spec.id.clone(), executable: Some(exe), cli, version, bundle_id: None, path: Some(path) });
            }
            None
        }
    }
}

/// The install of `spec`, or just its CLI on PATH.
pub fn detect(spec: &AppSpec, loc: &Locations) -> Option<Installed> {
    detect_install(spec, loc).or_else(|| find_cli(spec, None, loc).map(|cli| Installed { id: spec.id.clone(), path: None, executable: None, cli: Some(cli), version: None, bundle_id: None }))
}

/// Every known app, or app that describes itself, installed here.
pub fn discover(loc: &Locations) -> Vec<(AppSpec, Installed)> {
    all_specs(loc).into_iter().filter_map(|s| detect(&s, loc).map(|i| (s, i))).collect()
}

/// The self-description an app not in [`KNOWN`] ships in its install: `Contents/Resources/
/// otherwise-app.json` in a macOS bundle, next to the binary on Windows / Linux (SPEC §7.1).
pub const APP_MANIFEST: &str = "otherwise-app.json";
pub const MAX_APP_MANIFEST: u64 = 16 << 10;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppManifest {
    schema: u32,
    id: String,
    name: String,
    bundle_id: String,
    binary: String,
    #[serde(default)]
    cli: Vec<String>,
    #[serde(default)]
    mcp_args: Vec<String>,
    #[serde(default)]
    mcp_write_arg: Option<String>,
    #[serde(default)]
    open_source: bool,
    /// Unknown names are skipped (a newer app may declare more).
    #[serde(default)]
    capabilities: Vec<serde_json::Value>,
}

fn plain_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s != "." && s != ".." && !s.contains(['/', '\\', ':', '\0'])
}

fn ours(bundle_id: &str) -> bool {
    bundle_id.starts_with("io.github.otherwisetech.") || bundle_id.starts_with("com.otherwisetech.")
}

/// The [`AppSpec`] in an app's `otherwise-app.json`, read from `file`, for the install named
/// `app_name` (bundle name without `.app`) / `install_name` (folder). `None` if it is missing,
/// too large, unreadable, of a newer schema or invalid.
pub fn read_app_manifest(file: &Path, app_name: &str, install_name: &str) -> Option<AppSpec> {
    let meta = std::fs::metadata(file).ok()?;
    if !meta.is_file() || meta.len() > MAX_APP_MANIFEST {
        return None;
    }
    let m: AppManifest = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    if m.schema != 1 || !link::valid_app_id(&m.id) || m.name.trim().is_empty() || !ours(&m.bundle_id) || !plain_name(&m.binary) || !m.cli.iter().all(|c| plain_name(c)) {
        return None;
    }
    let capabilities = m.capabilities.iter().filter_map(|c| serde_json::from_value::<Capability>(c.clone()).ok()).collect();
    Some(AppSpec { id: m.id, name: m.name, bundle_id: m.bundle_id, app_name: app_name.into(), install_name: install_name.into(), binary: m.binary, cli: m.cli, mcp_args: m.mcp_args, mcp_write_arg: m.mcp_write_arg, open_source: m.open_source, capabilities })
}

/// Apps installed here that describe themselves ([`APP_MANIFEST`]) and aren't in [`KNOWN`]. On
/// macOS the bundle's `CFBundleIdentifier` must equal the manifest's bundle id. The first
/// install of an id wins (lookup order of [`Locations`]).
pub fn manifest_specs(loc: &Locations) -> Vec<AppSpec> {
    let mut out: Vec<AppSpec> = Vec::new();
    let add = |s: AppSpec, out: &mut Vec<AppSpec>| {
        if known(&s.id).is_none() && !out.iter().any(|o| o.id == s.id) {
            out.push(s);
        }
    };
    let sorted = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        v.sort();
        v
    };
    match loc.os {
        Os::Macos => {
            for dir in &loc.mac_dirs {
                for path in sorted(dir) {
                    let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".app")) else { continue };
                    let file = path.join("Contents").join("Resources").join(APP_MANIFEST);
                    let Some(spec) = read_app_manifest(&file, name, name) else { continue };
                    let bundle = read_info_plist(&path).and_then(|p| plist_string(&p, "CFBundleIdentifier"));
                    if bundle.as_deref() == Some(spec.bundle_id.as_str()) {
                        add(spec, &mut out);
                    }
                }
            }
        }
        Os::Windows | Os::Linux => {
            for root in &loc.roots {
                for path in sorted(root) {
                    let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
                    if let Some(spec) = read_app_manifest(&path.join(APP_MANIFEST), &name, &name) {
                        add(spec, &mut out);
                    }
                }
            }
        }
    }
    out
}

/// The manifest a built-in app's own install ships, if it is that app's: the bundle
/// `<appName>.app` (whose `CFBundleIdentifier` must be the known bundle id) / the folder
/// `<installName>` holding its binary, first install in lookup order, with the same id and
/// bundle id as the [`KNOWN`] entry.
fn known_install_manifest(k: &KnownApp, loc: &Locations) -> Option<AppSpec> {
    let m = match loc.os {
        Os::Macos => loc.mac_dirs.iter().map(|d| d.join(format!("{}.app", k.app_name))).find(|p| {
            p.join("Contents").is_dir() && read_info_plist(p).and_then(|x| plist_string(&x, "CFBundleIdentifier")).is_some_and(|b| b == k.bundle_id)
        }).and_then(|p| read_app_manifest(&p.join("Contents").join("Resources").join(APP_MANIFEST), k.app_name, k.install_name)),
        Os::Windows | Os::Linux => loc.roots.iter().map(|r| r.join(k.install_name)).find(|p| p.join(loc.exe_name(k.binary)).is_file())
            .and_then(|p| read_app_manifest(&p.join(APP_MANIFEST), k.app_name, k.install_name)),
    }?;
    (m.id == k.id && m.bundle_id == k.bundle_id).then_some(m)
}

/// `k`'s spec, with what the entry leaves empty (`cli`, `mcpArgs`, `mcpWriteArg`,
/// `capabilities`) taken from the [`APP_MANIFEST`] in its install (SPEC §7.1). A value the
/// table sets is never replaced, and nothing else (name, bundle id, binary…) is read.
pub fn known_spec_at(k: &KnownApp, loc: &Locations) -> AppSpec {
    let mut s = k.spec();
    if s.cli.is_empty() || s.mcp_args.is_empty() || s.mcp_write_arg.is_none() || s.capabilities.is_empty() {
        if let Some(m) = known_install_manifest(k, loc) {
            if s.cli.is_empty() {
                s.cli = m.cli;
            }
            if s.mcp_args.is_empty() {
                s.mcp_args = m.mcp_args;
            }
            if s.mcp_write_arg.is_none() {
                s.mcp_write_arg = m.mcp_write_arg;
            }
            if s.capabilities.is_empty() {
                s.capabilities = m.capabilities;
            }
        }
    }
    s
}

/// [`KNOWN`] (gaps filled from their installs, [`known_spec_at`]) plus [`manifest_specs`].
pub fn all_specs(loc: &Locations) -> Vec<AppSpec> {
    let mut v: Vec<AppSpec> = KNOWN.iter().map(|k| known_spec_at(k, loc)).collect();
    v.extend(manifest_specs(loc));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        let mut ids: Vec<&str> = KNOWN.iter().map(|a| a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), KNOWN.len(), "duplicate ids");
        for a in KNOWN {
            assert!(link::valid_app_id(a.id), "{}", a.id);
            assert!(a.bundle_id.starts_with("io.github.otherwisetech.") || a.bundle_id.starts_with("com.otherwisetech."), "{}", a.id);
            if a.capabilities.contains(&Mcp) {
                assert!(!a.cli.is_empty() && !a.mcp_args.is_empty(), "{} declares MCP without a CLI", a.id);
            }
        }
        assert_eq!(known("vigia").unwrap().spec().scheme().as_deref(), Some("otherwise-vigia"));
    }

    #[test]
    fn plist() {
        let xml = "<dict><key>CFBundleIdentifier</key>\n <string>io.github.otherwisetech.rama</string><key>N</key><string>A &amp; B</string><key>T</key><true/></dict>";
        assert_eq!(plist_string(xml, "CFBundleIdentifier").as_deref(), Some("io.github.otherwisetech.rama"));
        assert_eq!(plist_string(xml, "N").as_deref(), Some("A & B"));
        assert_eq!(plist_string(xml, "T"), None);
        assert_eq!(plist_string(xml, "missing"), None);
    }
}
