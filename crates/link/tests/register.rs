use otherwise_link::register::*;
use std::path::Path;

#[test]
fn macos_snippet_is_valid_plist_xml() {
    let s = macos_url_types("io.github.otherwisetech.vigia-app", &["otherwise-vigia"]);
    assert!(s.contains("<key>CFBundleURLTypes</key>"));
    assert!(s.contains("<string>otherwise-vigia</string>"));
    assert!(s.contains("<string>io.github.otherwisetech.vigia-app</string>"));
    assert_eq!(s.matches("<dict>").count(), s.matches("</dict>").count());
    assert_eq!(s.matches("<array>").count(), s.matches("</array>").count());
    let two = macos_url_types("x", &["otherwise", "otherwise-apps"]);
    assert!(two.contains("<array><string>otherwise</string><string>otherwise-apps</string></array>"));
}

#[test]
fn linux_desktop_patch_adds_mime_and_arg() {
    let src = "[Desktop Entry]\nType=Application\nName=Vigía\nExec=vigia-app\n\n[Desktop Action new]\nExec=vigia-app --new\n";
    let out = linux_desktop_patch(src, &["otherwise-vigia"]);
    assert!(out.contains("Exec=vigia-app %u\n"), "{out}");
    assert!(out.contains("MimeType=x-scheme-handler/otherwise-vigia;\n[Desktop Action new]"), "{out}");
    // other groups are left alone
    assert!(out.contains("Exec=vigia-app --new\n"));
    // idempotent, and existing MimeType entries are kept
    assert_eq!(linux_desktop_patch(&out, &["otherwise-vigia"]), out);
    let with = "[Desktop Entry]\nExec=app %F\nMimeType=application/pdf;\n";
    assert_eq!(linux_desktop_patch(with, &["otherwise-expense"]), "[Desktop Entry]\nExec=app %F\nMimeType=application/pdf;x-scheme-handler/otherwise-expense;\n");
    assert_eq!(linux_mime_types(&["a", "b"]), "x-scheme-handler/a;x-scheme-handler/b;");
}

#[test]
fn windows_registry() {
    let exe = Path::new(r"C:\Users\me\AppData\Local\Programs\Otherwise\vigia\vigia-app.exe");
    let cmds = windows_reg_commands("otherwise-vigia", exe, "Vigía");
    assert_eq!(cmds[0], ["add", r"HKCU\Software\Classes\otherwise-vigia", "/ve", "/d", "URL:Vigía", "/f"]);
    assert_eq!(cmds[1][3], "URL Protocol");
    assert_eq!(cmds[3][1], r"HKCU\Software\Classes\otherwise-vigia\shell\open\command");
    assert_eq!(cmds[3][4], format!("\"{}\" \"%1\"", exe.display()));
    let reg = windows_reg_file("otherwise-vigia", exe, "Vigía");
    assert!(reg.starts_with("Windows Registry Editor Version 5.00\r\n"));
    assert!(reg.contains(r#"@="\"C:\\Users\\me\\AppData\\Local\\Programs\\Otherwise\\vigia\\vigia-app.exe\" \"%1\"""#), "{reg}");
    assert!(valid_scheme("otherwise-vigia"));
    assert!(!valid_scheme("Otherwise"));
    assert!(!valid_scheme("1x"));
    assert!(!valid_scheme(""));
}
