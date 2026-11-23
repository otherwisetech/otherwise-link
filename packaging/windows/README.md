# Windows installer template

A per-user NSIS installer for the Otherwise desktop apps. One small `app.toml` per app, one
script, and the app's existing Windows build (the release zip) in; out comes
`<app>-<version>-windows-x86_64-setup.exe` and its `.sha256`.

```
packaging/windows/
  installer.nsi        the installer (all logic; app-specific parts come from app.nsh)
  lang.nsh             the template's texts: English, Spanish, Portuguese (BR + PT), French
  render.py            app.toml + payload -> app.nsh (Python 3.11+, standard library only)
  build-installer.sh   render + makensis -WX + .sha256 (macOS, Linux, Git Bash)
  examples/expense/    app.toml for expense (right-click "Add to expense", replaces the .reg files)
  examples/vigia/      app.toml for Vigía (two exes, two licenses, otherwise-vigia:// protocol)
  tests/test_render.py config -> .nsi rendering, staging, and a full reproducible build
```

## What the installer does

- **Per user, no admin** (`RequestExecutionLevel user`). Installs to
  `%LOCALAPPDATA%\Programs\<name>` (the user can pick another folder on first install).
- **Start menu** shortcut; **desktop** shortcut as a checkbox (default per app).
- **Apps & features** entry under `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<id>`:
  DisplayName, DisplayVersion, Publisher "Otherwise Technologies LLC", URLInfoAbout
  https://otherwisetech.com, DisplayIcon, EstimatedSize, InstallLocation, (Quiet)UninstallString.
- **App Paths** (`HKCU\…\App Paths\<exe>`) so Win+R `expense` works.
- Optional per app: **URL protocols** (`HKCU\Software\Classes\<scheme>`, the same values
  `otherwise_link::register::register_windows` writes), **file associations** (ProgID +
  `OpenWithProgids`; the default handler is claimed only if the extension has none),
  **Explorer context-menu entries** (files, folders, folder background), **start at sign-in**
  (`HKCU\…\Run\<id>`, a checkbox, off by default; for the Otherwise Helper).
- **Upgrades in place**: the folder comes from the previous install, the folder page is
  skipped, the previous checkbox choices are kept. If the app is running (its exe is locked),
  setup asks to close it (a normal close request, never a forced kill), then offers Retry.
  Silent mode asks once and exits with code 2 if the app is still open.
- **Uninstall** removes exactly the files it installed (no recursive delete of the folder),
  the shortcuts and the registry entries. Protocol, association and context-menu keys are
  removed only while they still point at this install. It **never deletes the user's data**:
  `%USERPROFILE%\.otherwise` (or wherever the data root was moved) and the app's own folders
  stay, and the uninstall page says so.
- **Languages**: picked from the Windows display language; English when it is none of
  en / es / pt-BR / pt / fr.
- **License page** with the app's license file(s); the **app icon** on the installer, the
  uninstaller, the shortcuts and Apps & features.
- **Reproducible**: same inputs give a byte-identical setup.exe (no file times stored, solid
  LZMA, sorted file list). Checked by the test.
- Silent install / uninstall: `setup.exe /S`, `uninstall.exe /S` (winget and the Helper use
  these). `/D=C:\path` (last argument) picks the folder on a first install.

## app.toml

Paths are relative to the file (or to `--root`). Only `[app]` is required.

```toml
[app]
id = "otherwise-expense"          # registry key: never change it once shipped
artifact = "expense"              # file name: expense-0.6.0-windows-x86_64-setup.exe (default: id)
name = "expense"                  # folder, Start menu, Apps & features
description = "Receipt organizer"
exe = "expense.exe"               # main exe, inside the payload
version_from = "../../Cargo.toml" # [workspace.package] or [package] version; or `version = "1.2.3"`
icon = "../../crates/app/assets/logo/AppIcon.ico"
license = "../../LICENSE"         # or a list: ["../../LICENSE-MIT", "../../LICENSE-APACHE"]
# publisher / url default to Otherwise Technologies LLC / https://otherwisetech.com

[files]                           # which payload files to install (globs; default all)
include = ["expense.exe", "README.md"]
exclude = []
extra = [{ src = "../../docs/guide.pdf", dest = "guide.pdf" }]   # from the repo

[shortcuts]
desktop_default = true            # desktop checkbox ticked by default
extra = [{ name = "Voltia Calc", exe = "voltia-calc.exe", args = "" }]   # more Start menu entries

[[protocols]]
scheme = "otherwise-vigia"
description = "Vigía"

[[file_associations]]
extensions = [".ged", ".gedcom"]
prog_id = "Otherwise.Genealogy.1"
description = "GEDCOM family tree"
args = '"%1"'

[[context_menu]]
id = "AddToExpense"
targets = ["files", "folders"]    # also "folder_background"
label = { en = "Add to expense", es = "Añadir a expense", pt = "Adicionar ao expense", fr = "Ajouter à expense" }
args = '"%1"'

[autostart]
enabled = true                    # show the "Start when I sign in" checkbox
default = false
args = "--background"
```

On Windows 11 context-menu entries appear under *Show more options* (the new menu only takes
packaged apps).

## Build it locally

```bash
brew install makensis              # or: sudo apt-get install nsis
packaging/windows/build-installer.sh \
  --config packaging/windows/app.toml \
  --payload dist/expense-windows-x86_64.zip \   # the release zip, the .exe, or a folder
  --version 0.6.0 --out dist
```

A zip with one top folder (what `Compress-Archive -Path dist/$pkg` makes) is unpacked without
it. Tests: `python3.11 -I -m unittest discover -s packaging/windows/tests -v` (the build test
runs when `makensis` is on PATH).

## Adopt it in an app

1. **Get the template**, either way:
   - *Vendor*: copy `installer.nsi`, `lang.nsh`, `render.py`, `build-installer.sh` into
     `packaging/windows/installer/` of the app and note the otherwise-link commit in a comment.
     Simple, works offline, updates are a manual copy.
   - *Fetch at a pinned ref* in CI (below). Pin a commit SHA, or a tag once an otherwise-link
     release contains `packaging/windows` (its tags follow the crate version).
2. Write `packaging/windows/app.toml` (start from `examples/`). Remove the app's `.reg` files
   from the zip step once the installer ships.
3. Add a job to `.github/workflows/release.yml` that runs after the Windows build, on Linux:

```yaml
  windows-installer:
    needs: build
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/checkout@v4
        with:
          repository: otherwisetech/otherwise-link
          ref: <commit sha or tag>
          path: .otherwise-link
          sparse-checkout: packaging/windows
      - uses: actions/download-artifact@v4
        with:
          name: expense-windows-x86_64
          path: dist
      - run: sudo apt-get update && sudo apt-get install -y --no-install-recommends nsis
      - run: |
          .otherwise-link/packaging/windows/build-installer.sh \
            --config packaging/windows/app.toml \
            --payload dist/expense-windows-x86_64.zip \
            --version "${GITHUB_REF_NAME#v}" --out dist
      - uses: actions/upload-artifact@v4
        with:
          name: expense-windows-installer
          path: dist/*-setup.exe*
```

   and attach `dist/*-setup.exe` and `*.sha256` to the release next to the zip (keep the zip for
   people who want a portable copy). Ubuntu 24.04's NSIS is 3.09; anything 3.08+ works.
4. Smoke-test once on a real Windows machine: install, open, right-click / link / association,
   upgrade over it with the next version, uninstall, check `%USERPROFILE%\.otherwise` is intact.

## SmartScreen: unsigned for now

The installer and the exe are not code-signed yet. Windows shows *Windows protected your PC*
for a downloaded unsigned setup.exe: **More info → Run anyway**. Say so on the download page.
Per-user install means no UAC prompt either way.

## Signing later

Sign both the app exe and the installer:

- **The exe**: in the Windows build job, before zipping (`signtool sign /fd sha256 /tr
  http://timestamp.acs.microsoft.com /td sha256 …`, or the `azure/trusted-signing-action` for
  Azure Trusted Signing). The installer then carries the signed exe.
- **The installer and its uninstaller**: `build-installer.sh --sign-cmd "<command>"`. The
  command gets the file path as its last argument and runs twice: on the uninstaller while
  makensis builds it (`!uninstfinalize`) and on the finished setup.exe (`!finalize`). From
  Linux that is `jsign` (supports Azure Trusted Signing and PKCS#11 tokens) or `osslsigncode`;
  on a Windows runner `signtool`. Sign before computing the `.sha256` (the script already does
  it in that order). Signing makes the output depend on the timestamp, so signed builds are no
  longer byte-reproducible; the unsigned payload still is.

## winget later

Once installers are on GitHub releases, a manifest in `microsoft/winget-pkgs`
(`manifests/o/OtherwiseTechnologies/expense/0.6.0/`) needs:

```yaml
PackageIdentifier: OtherwiseTechnologies.expense
PackageVersion: 0.6.0
InstallerType: nullsoft          # /S and /D= are implied
Scope: user
InstallModes: [interactive, silent]
UpgradeBehavior: install
ProductCode: otherwise-expense   # = app.id, the Uninstall key
Installers:
  - Architecture: x64
    InstallerUrl: https://github.com/otherwisetech/expense-rs/releases/download/v0.6.0/expense-0.6.0-windows-x86_64-setup.exe
    InstallerSha256: <from the .sha256>
```

`wingetcreate update` can bump it from the release job. winget's validation pipeline runs
SmartScreen/Defender checks; signing first makes acceptance smoother. Private-repo apps need
their release assets public (or another download host) for winget.

## Not covered (yet)

- Machine-wide installs (would need admin and HKLM).
- Adding a CLI exe to `PATH` (needs a plugin or PowerShell; App Paths covers Win+R only).
- Downgrade protection: installing an older version over a newer one is allowed.
- The installer stub is 32-bit (NSIS's x86-unicode); it installs the 64-bit app normally and
  the HKCU keys it writes are not redirected.
