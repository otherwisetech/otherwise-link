//! Making the OS hand an app its links. Generating the snippets is pure (and tested); the
//! `register_*` runners are thin.
//!
//! - macOS: the bundle's `Info.plist` declares `CFBundleURLTypes` ([`macos_url_types`]); bundle
//!   scripts paste it in, LaunchServices registers it when the app is copied / `lsregister`ed.
//!   The URL arrives as an Apple event (GPUI: `Application::on_open_urls`).
//! - Linux: the `.desktop` file lists `MimeType=x-scheme-handler/<scheme>;` and its `Exec` takes
//!   `%u` ([`linux_desktop_patch`]); [`register_linux`] makes it the default handler with
//!   `xdg-mime`. The URL arrives as an argument.
//! - Windows: `HKCU\Software\Classes\<scheme>` with `URL Protocol` and
//!   `shell\open\command = "exe" "%1"` ([`windows_reg_commands`], or a `.reg` file from
//!   [`windows_reg_file`]); per user, no admin rights. The URL arrives as an argument.
use std::io;
use std::path::Path;
use std::process::Command;

/// `[a-z][a-z0-9+.-]*` (RFC 3986, lower case only).
pub fn valid_scheme(s: &str) -> bool {
    let mut b = s.bytes();
    b.next().is_some_and(|c| c.is_ascii_lowercase()) && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'+' | b'.' | b'-'))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The `CFBundleURLTypes` key + array for an `Info.plist` `<dict>`.
pub fn macos_url_types(url_name: &str, schemes: &[&str]) -> String {
    let list: String = schemes.iter().map(|s| format!("<string>{}</string>", xml_escape(s))).collect();
    format!(
        "  <key>CFBundleURLTypes</key>\n  <array>\n    <dict>\n      <key>CFBundleURLName</key><string>{}</string>\n      <key>CFBundleTypeRole</key><string>Viewer</string>\n      <key>CFBundleURLSchemes</key><array>{list}</array>\n    </dict>\n  </array>\n",
        xml_escape(url_name)
    )
}

/// `x-scheme-handler/a;x-scheme-handler/b;`
pub fn linux_mime_types(schemes: &[&str]) -> String {
    schemes.iter().map(|s| format!("x-scheme-handler/{s};")).collect()
}

/// `desktop` with the scheme handlers added to `MimeType=` (kept if already there) and `%u`
/// appended to `Exec=` when it takes no file / URL argument yet.
pub fn linux_desktop_patch(desktop: &str, schemes: &[&str]) -> String {
    let mut out = Vec::new();
    let mut saw_mime = false;
    let mut in_entry = false;
    for line in desktop.lines() {
        let t = line.trim_start();
        if t.starts_with('[') {
            if in_entry && !saw_mime {
                out.push(format!("MimeType={}", linux_mime_types(schemes)));
                saw_mime = true;
            }
            in_entry = t == "[Desktop Entry]";
            out.push(line.to_string());
            continue;
        }
        if in_entry && let Some(v) = t.strip_prefix("MimeType=") {
            let mut parts: Vec<String> = v.split(';').filter(|p| !p.is_empty()).map(str::to_string).collect();
            for s in schemes {
                let m = format!("x-scheme-handler/{s}");
                if !parts.contains(&m) {
                    parts.push(m);
                }
            }
            out.push(format!("MimeType={};", parts.join(";")));
            saw_mime = true;
        } else if in_entry && let Some(v) = t.strip_prefix("Exec=") {
            let takes_arg = ["%u", "%U", "%f", "%F"].iter().any(|c| v.contains(c));
            out.push(if takes_arg { line.to_string() } else { format!("Exec={v} %u") });
        } else {
            out.push(line.to_string());
        }
    }
    if in_entry && !saw_mime {
        out.push(format!("MimeType={}", linux_mime_types(schemes)));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Makes `desktop_file` (a name in `~/.local/share/applications`, like
/// `io.github.otherwisetech.vigia-app.desktop`) the default handler of each scheme.
pub fn register_linux(desktop_file: &str, schemes: &[&str]) -> io::Result<()> {
    for s in schemes {
        let st = Command::new("xdg-mime").args(["default", desktop_file, &format!("x-scheme-handler/{s}")]).status()?;
        if !st.success() {
            return Err(io::Error::other(format!("xdg-mime failed for {s}")));
        }
    }
    let apps = crate::paths::home().join(".local").join("share").join("applications");
    let _ = Command::new("update-desktop-database").arg(apps).status();
    Ok(())
}

/// `reg add` argument lists (run each as `reg <args…>`) registering `scheme` for the current user.
pub fn windows_reg_commands(scheme: &str, exe: &Path, description: &str) -> Vec<Vec<String>> {
    let key = format!("HKCU\\Software\\Classes\\{scheme}");
    let exe = exe.display().to_string();
    let v = |s: &str| s.to_string();
    vec![
        vec![v("add"), key.clone(), v("/ve"), v("/d"), format!("URL:{description}"), v("/f")],
        vec![v("add"), key.clone(), v("/v"), v("URL Protocol"), v("/d"), v(""), v("/f")],
        vec![v("add"), format!("{key}\\DefaultIcon"), v("/ve"), v("/d"), format!("\"{exe}\",0"), v("/f")],
        vec![v("add"), format!("{key}\\shell\\open\\command"), v("/ve"), v("/d"), format!("\"{exe}\" \"%1\""), v("/f")],
    ]
}

/// The same as a `.reg` file (for installers).
pub fn windows_reg_file(scheme: &str, exe: &Path, description: &str) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let exe = exe.display().to_string();
    let key = format!("HKEY_CURRENT_USER\\Software\\Classes\\{scheme}");
    format!(
        "Windows Registry Editor Version 5.00\r\n\r\n[{key}]\r\n@=\"URL:{}\"\r\n\"URL Protocol\"=\"\"\r\n\r\n[{key}\\DefaultIcon]\r\n@=\"{}\"\r\n\r\n[{key}\\shell\\open\\command]\r\n@=\"{}\"\r\n",
        esc(description),
        esc(&format!("\"{exe}\",0")),
        esc(&format!("\"{exe}\" \"%1\""))
    )
}

pub fn register_windows(scheme: &str, exe: &Path, description: &str) -> io::Result<()> {
    if !valid_scheme(scheme) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid scheme"));
    }
    for args in windows_reg_commands(scheme, exe, description) {
        let st = Command::new("reg").args(&args).stdout(std::process::Stdio::null()).status()?;
        if !st.success() {
            return Err(io::Error::other(format!("reg {} failed", args.join(" "))));
        }
    }
    Ok(())
}
