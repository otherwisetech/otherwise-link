#!/usr/bin/env python3
"""Render an app's `app.toml` into the NSIS include `app.nsh` that `installer.nsi` builds.

    python3 render.py --config app.toml --payload <zip|exe|dir> --stage <empty dir>
                      [--root DIR] [--version X.Y.Z] [--out DIR]

Copies the payload into <stage>/payload (a zip is extracted; a single top-level folder is
stripped), writes <stage>/LICENSE.txt and <stage>/app.nsh, and prints the setup file name.
Relative paths in app.toml resolve against --root (default: the folder holding app.toml).
Needs Python 3.11+ (tomllib). Standard library only.
"""

from __future__ import annotations

import argparse
import fnmatch
import os
import re
import shutil
import sys
import zipfile
from pathlib import Path, PurePosixPath

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    sys.exit("render.py needs Python 3.11+ (tomllib)")

PUBLISHER = "Otherwise Technologies LLC"
URL = "https://otherwisetech.com"
# Installer languages (NSIS names) and the config key that feeds each one.
LANGS = [("ENGLISH", "en"), ("SPANISH", "es"), ("PORTUGUESEBR", "pt"), ("PORTUGUESE", "pt"), ("FRENCH", "fr")]
TARGETS = {"files": "*", "folders": "Directory", "folder_background": "Directory\\Background"}

RE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
RE_EXE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._ -]*\.exe$")
RE_SCHEME = re.compile(r"^[a-z][a-z0-9+.-]*$")
RE_EXT = re.compile(r"^\.[A-Za-z0-9_-]+$")
RE_PROGID = re.compile(r"^[A-Za-z][A-Za-z0-9.]*$")
RE_VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.+-]*)?$")


class ConfigError(ValueError):
    pass


# ---------------------------------------------------------------- NSIS quoting

def lit(s: str) -> str:
    """Text as the inside of an NSIS double-quoted string (no variables expand)."""
    if any(c in s for c in "\0"):
        raise ConfigError(f"NUL in {s!r}")
    return (s.replace("$", "$$").replace('"', '$\\"').replace("\r", "$\\r")
            .replace("\n", "$\\n").replace("\t", "$\\t"))


def q(*parts: str) -> str:
    """An NSIS string from pieces: str pieces are literal text, Raw pieces are NSIS code."""
    return '"' + "".join(p if isinstance(p, Raw) else lit(p) for p in parts) + '"'


class Raw(str):
    pass


INSTDIR = Raw("$INSTDIR")


def compile_path(p: Path) -> str:
    s = str(p)
    if any(c in s for c in '"$\n\r'):
        raise ConfigError(f"unsupported character in path {s}")
    return '"' + s + '"'


def win(rel: str) -> str:
    return rel.replace("/", "\\")


def exe_cmd(exe: str, args: str = "") -> list:
    """`"$INSTDIR\\exe" args` as q() pieces."""
    pieces = ['"', INSTDIR, "\\" + win(exe) + '"']
    if args:
        pieces.append(" " + args)
    return pieces


# ---------------------------------------------------------------- config

def need(d: dict, key: str, where: str):
    if key not in d or d[key] in ("", None):
        raise ConfigError(f"{where}.{key} is required")
    return d[key]


def check(regex: re.Pattern, value: str, what: str) -> str:
    if not isinstance(value, str) or not regex.match(value):
        raise ConfigError(f"invalid {what}: {value!r}")
    return value


def localized(v, where: str) -> dict:
    """"text" or { en = "...", es = "..." } -> {key: text} for every installer language."""
    if isinstance(v, str):
        v = {"en": v}
    if not isinstance(v, dict) or not v.get("en"):
        raise ConfigError(f"{where} needs an English text (a string or a table with `en`)")
    return {key: v.get(key) or v["en"] for _, key in LANGS}


def cargo_version(path: Path) -> str:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    v = data.get("workspace", {}).get("package", {}).get("version") or data.get("package", {}).get("version")
    if not isinstance(v, str):
        raise ConfigError(f"no [workspace.package] or [package] version in {path}")
    return v


def quad(version: str) -> str:
    m = RE_VERSION.match(version)
    if not m:
        raise ConfigError(f"version must look like 1.2.3 (got {version!r})")
    return ".".join(m.groups()) + ".0"


def load(config_path: Path, root: Path | None = None, version: str | None = None) -> dict:
    raw = tomllib.loads(config_path.read_text(encoding="utf-8"))
    base = Path(os.path.abspath(root or config_path.parent))
    rp = lambda rel: Path(os.path.normpath(base / rel))  # noqa: E731 (lexical: root may not exist yet)
    app = raw.get("app", {})
    c: dict = {}
    c["id"] = check(RE_ID, need(app, "id", "app"), "app.id")
    c["name"] = need(app, "name", "app")
    if any(ch in c["name"] for ch in '\\/:*?"<>|'):
        raise ConfigError("app.name is also the folder and shortcut name: no \\ / : * ? \" < > |")
    c["exe"] = check(RE_EXE, need(app, "exe", "app"), "app.exe")
    c["artifact"] = check(RE_ID, app.get("artifact", c["id"]), "app.artifact")
    c["description"] = app.get("description", c["name"])
    c["publisher"] = app.get("publisher", PUBLISHER)
    c["url"] = app.get("url", URL)

    if version is None:
        if app.get("version"):
            version = app["version"]
        elif app.get("version_from"):
            version = cargo_version(rp(app["version_from"]))
        else:
            raise ConfigError("set app.version or app.version_from (a Cargo.toml), or pass --version")
    c["version"] = version
    c["version_quad"] = quad(version)

    c["icon"] = rp(need(app, "icon", "app"))
    licenses = app.get("license", [])
    if isinstance(licenses, str):
        licenses = [licenses]
    if not licenses:
        raise ConfigError("app.license is required (one path or a list)")
    c["licenses"] = [rp(p) for p in licenses]

    files = raw.get("files", {})
    c["include"] = files.get("include", ["**"])
    c["exclude"] = files.get("exclude", [])
    c["extra"] = []
    for i, e in enumerate(files.get("extra", [])):
        src = need(e, "src", f"files.extra[{i}]")
        dest = e.get("dest", PurePosixPath(src).name)
        safe_rel(dest)
        c["extra"].append((rp(src), dest))

    sc = raw.get("shortcuts", {})
    c["desktop_default"] = bool(sc.get("desktop_default", False))
    c["shortcuts"] = []
    for i, s in enumerate(sc.get("extra", [])):
        name = need(s, "name", f"shortcuts.extra[{i}]")
        if any(ch in name for ch in '\\/:*?"<>|'):
            raise ConfigError(f"invalid shortcut name {name!r}")
        c["shortcuts"].append({"name": name, "exe": check(RE_EXE, need(s, "exe", f"shortcuts.extra[{i}]"), "shortcut exe"),
                               "args": s.get("args", "")})

    c["protocols"] = []
    for i, p in enumerate(raw.get("protocols", [])):
        c["protocols"].append({"scheme": check(RE_SCHEME, need(p, "scheme", f"protocols[{i}]"), "URL scheme"),
                               "description": p.get("description", c["name"])})

    c["associations"] = []
    for i, a in enumerate(raw.get("file_associations", [])):
        exts = a.get("extensions") or [need(a, "extension", f"file_associations[{i}]")]
        c["associations"].append({
            "extensions": [check(RE_EXT, e.lower(), "file extension") for e in exts],
            "prog_id": check(RE_PROGID, need(a, "prog_id", f"file_associations[{i}]"), "prog_id"),
            "description": a.get("description", c["name"]),
            "args": a.get("args", '"%1"'),
        })

    c["verbs"] = []
    for i, v in enumerate(raw.get("context_menu", [])):
        where = f"context_menu[{i}]"
        targets = v.get("targets", ["files"])
        for t in targets:
            if t not in TARGETS:
                raise ConfigError(f"{where}.targets: {t!r} is not one of {', '.join(TARGETS)}")
        c["verbs"].append({"id": check(RE_ID, need(v, "id", where), "context_menu id"),
                           "label": localized(need(v, "label", where), where + ".label"),
                           "targets": targets, "args": v.get("args", '"%1"')})

    au = raw.get("autostart", {})
    c["autostart"] = bool(au.get("enabled", False))
    c["autostart_default"] = bool(au.get("default", False))
    c["autostart_args"] = au.get("args", "")
    return c


# ---------------------------------------------------------------- payload

def safe_rel(rel: str) -> PurePosixPath:
    p = PurePosixPath(rel.replace("\\", "/"))
    if p.is_absolute() or not p.parts or any(part in ("..", "") for part in p.parts) or ":" in rel:
        raise ConfigError(f"unsafe path {rel!r}")
    return p


def stage_payload(src: Path, dest: Path, exe: str) -> None:
    """Copy the build output (zip, exe or folder) into `dest`."""
    dest.mkdir(parents=True)
    if src.is_dir():
        shutil.copytree(src, dest, dirs_exist_ok=True)
    elif src.suffix.lower() == ".zip":
        with zipfile.ZipFile(src) as z:
            names = [n for n in z.namelist() if not n.endswith("/")]
            for n in names:
                safe_rel(n)
            tops = {PurePosixPath(n).parts[0] for n in names}
            strip = len(tops) == 1 and all(len(PurePosixPath(n).parts) > 1 for n in names)
            for n in names:
                rel = PurePosixPath(*PurePosixPath(n).parts[(1 if strip else 0):])
                out = dest.joinpath(*rel.parts)
                out.parent.mkdir(parents=True, exist_ok=True)
                with z.open(n) as r, open(out, "wb") as w:
                    shutil.copyfileobj(r, w)
    elif src.suffix.lower() == ".exe":
        shutil.copy2(src, dest / exe)
    else:
        raise ConfigError(f"payload must be a .zip, an .exe or a folder: {src}")


def payload_files(payload: Path, include: list, exclude: list) -> list[str]:
    out = []
    for p in sorted(payload.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(payload).as_posix()
        if any(fnmatch.fnmatch(rel, g) for g in include) and not any(fnmatch.fnmatch(rel, g) for g in exclude):
            out.append(rel)
    return out


def write_license(paths: list[Path], dest: Path) -> None:
    parts = []
    for p in paths:
        parts.append(p.read_text(encoding="utf-8").replace("\r\n", "\n").strip("\n"))
    text = ("\n\n" + "-" * 72 + "\n\n").join(parts) + "\n"
    dest.write_bytes(b"\xef\xbb\xbf" + text.replace("\n", "\r\n").encode("utf-8"))


# ---------------------------------------------------------------- rendering

def render(c: dict, files: list[tuple[Path, str]], license_txt: Path, outfile: Path) -> str:
    """The app.nsh text. `files` = (source, install-relative posix path), already sorted."""
    L: list[str] = ["; Generated by otherwise-link packaging/windows/render.py. Do not edit.", ""]
    d = lambda k, v: L.append(f"!define {k} {v}")  # noqa: E731
    d("APP_ID", q(c["id"]))
    d("APP_NAME", q(c["name"]))
    d("APP_EXE", q(c["exe"]))
    d("APP_DESCRIPTION", q(c["description"]))
    d("APP_VERSION", q(c["version"]))
    d("APP_VERSION_QUAD", q(c["version_quad"]))
    d("APP_PUBLISHER", q(c["publisher"]))
    d("APP_URL", q(c["url"]))
    d("APP_ICON", compile_path(c["icon"]))
    d("APP_LICENSE", compile_path(license_txt))
    d("APP_OUTFILE", compile_path(outfile))
    size = sum(src.stat().st_size for src, _ in files) + c["icon"].stat().st_size
    d("APP_ESTIMATED_SIZE_KB", str((size + 1023) // 1024))
    d("APP_DESKTOP_DEFAULT", "1" if c["desktop_default"] else "0")
    if c["autostart"]:
        d("APP_AUTOSTART", "1")
        d("APP_AUTOSTART_DEFAULT", "1" if c["autostart_default"] else "0")
    L.append("")

    names = [rel for _, rel in files]
    if c["exe"] not in names:
        raise ConfigError(f"{c['exe']} is not in the payload (files: {', '.join(names) or 'none'})")
    for s in c["shortcuts"]:
        if s["exe"] not in names:
            raise ConfigError(f"shortcut exe {s['exe']} is not in the payload")
    if len(set(n.lower() for n in names)) != len(names) or c["id"].lower() + ".ico" in (n.lower() for n in names) \
            or "uninstall.exe" in (n.lower() for n in names):
        raise ConfigError("duplicate file names in the payload (or one named <id>.ico / uninstall.exe)")

    L.append("!macro APP_INSTALL_FILES")
    current = None
    for src, rel in files:
        parent = str(PurePosixPath(rel).parent)
        if parent != current:
            L.append("  SetOutPath " + q(INSTDIR, "" if parent == "." else "\\" + win(parent)))
            current = parent
        L.append(f"  File /oname={PurePosixPath(rel).name} {compile_path(src)}")
    L.append('  SetOutPath "$INSTDIR"')
    L.append(f"  File /oname={c['id']}.ico \"${{APP_ICON}}\"")
    L.append("!macroend")
    L.append("")

    L.append("!macro APP_UNINSTALL_FILES")
    for rel in names:
        L.append("  Delete " + q(INSTDIR, "\\" + win(rel)))
    L.append("  Delete " + q(INSTDIR, f"\\{c['id']}.ico"))
    dirs = sorted({str(p) for rel in names for p in PurePosixPath(rel).parents if str(p) != "."},
                  key=lambda s: (-s.count("/"), s))
    for dname in dirs:
        L.append("  RMDir " + q(INSTDIR, "\\" + win(dname)))
    L.append("!macroend")
    L.append("")

    exes = [n for n in names if n.lower().endswith(".exe")]
    L.append("!macro APP_ENSURE_CLOSED PREFIX")
    for e in exes:
        L.append("  Push " + q(INSTDIR, "\\" + win(e)))
        L.append("  Call ${PREFIX}OwEnsureClosed")
    L.append("!macroend")
    L.append("")

    icon = [INSTDIR, f"\\{c['id']}.ico"]
    hk = "HKCU"
    L.append("!macro APP_REGISTER")
    for p in c["protocols"]:
        key = f"Software\\Classes\\{p['scheme']}"
        L.append(f"  WriteRegStr {hk} {q(key)} \"\" {q('URL:' + p['description'])}")
        L.append(f"  WriteRegStr {hk} {q(key)} \"URL Protocol\" \"\"")
        L.append(f"  WriteRegStr {hk} {q(key + chr(92) + 'DefaultIcon')} \"\" {q(*exe_cmd(c['exe']), ',0')}")
        L.append(f"  WriteRegStr {hk} {q(key + chr(92) + 'shell' + chr(92) + 'open' + chr(92) + 'command')} \"\" "
                 f"{q(*exe_cmd(c['exe'], chr(34) + '%1' + chr(34)))}")
    for a in c["associations"]:
        pk = f"Software\\Classes\\{a['prog_id']}"
        L.append(f"  WriteRegStr {hk} {q(pk)} \"\" {q(a['description'])}")
        L.append(f"  WriteRegStr {hk} {q(pk + chr(92) + 'DefaultIcon')} \"\" {q(*icon)}")
        L.append(f"  WriteRegStr {hk} {q(pk + chr(92) + 'shell' + chr(92) + 'open' + chr(92) + 'command')} \"\" "
                 f"{q(*exe_cmd(c['exe'], a['args']))}")
        for ext in a["extensions"]:
            ek = f"Software\\Classes\\{ext}"
            L.append(f"  WriteRegStr {hk} {q(ek + chr(92) + 'OpenWithProgids')} {q(a['prog_id'])} \"\"")
            # Claim the default only when nothing else has; Windows asks the user otherwise.
            L.append(f"  !insertmacro OW_CLAIM_DEFAULT {q(ek)} {q(a['prog_id'])}")
    for v in c["verbs"]:
        for t in v["targets"]:
            key = f"Software\\Classes\\{TARGETS[t]}\\shell\\{v['id']}"
            args = v["args"].replace("%1", "%V") if t == "folder_background" else v["args"]
            L.append(f"  WriteRegStr {hk} {q(key)} \"\" \"$(APP_VERB_{v['id'].replace('-', '_').replace('.', '_')})\"")
            L.append(f"  WriteRegStr {hk} {q(key)} \"Icon\" {q(*icon)}")
            L.append(f"  WriteRegStr {hk} {q(key + chr(92) + 'command')} \"\" {q(*exe_cmd(c['exe'], args))}")
    L.append("!macroend")
    L.append("")

    L.append("!macro APP_UNREGISTER")
    for p in c["protocols"]:
        key = f"Software\\Classes\\{p['scheme']}"
        # Only when it still opens this install: another app (or a newer copy) may own it now.
        L.append(f"  !insertmacro OW_DELETE_KEY_IF_OURS {q(key)} {q(key + chr(92) + 'shell' + chr(92) + 'open' + chr(92) + 'command')} "
                 f"{q(*exe_cmd(c['exe'], chr(34) + '%1' + chr(34)))}")
    for a in c["associations"]:
        pk = f"Software\\Classes\\{a['prog_id']}"
        for ext in a["extensions"]:
            ek = f"Software\\Classes\\{ext}"
            L.append(f"  DeleteRegValue {hk} {q(ek + chr(92) + 'OpenWithProgids')} {q(a['prog_id'])}")
            L.append(f"  !insertmacro OW_RELEASE_DEFAULT {q(ek)} {q(a['prog_id'])}")
        L.append(f"  !insertmacro OW_DELETE_KEY_IF_OURS {q(pk)} {q(pk + chr(92) + 'shell' + chr(92) + 'open' + chr(92) + 'command')} "
                 f"{q(*exe_cmd(c['exe'], a['args']))}")
    for v in c["verbs"]:
        for t in v["targets"]:
            key = f"Software\\Classes\\{TARGETS[t]}\\shell\\{v['id']}"
            args = v["args"].replace("%1", "%V") if t == "folder_background" else v["args"]
            L.append(f"  !insertmacro OW_DELETE_KEY_IF_OURS {q(key)} {q(key + chr(92) + 'command')} {q(*exe_cmd(c['exe'], args))}")
    L.append("!macroend")
    L.append("")

    L.append("!macro APP_EXTRA_SHORTCUTS")
    for s in c["shortcuts"]:
        L.append(f"  CreateShortcut {q(Raw('$SMPROGRAMS'), chr(92) + s['name'] + '.lnk')} {q(INSTDIR, chr(92) + win(s['exe']))} "
                 f"{q(s['args'])} {q(INSTDIR, chr(92) + win(s['exe']))} 0")
    L.append("!macroend")
    L.append("!macro APP_REMOVE_EXTRA_SHORTCUTS")
    for s in c["shortcuts"]:
        L.append(f"  Delete {q(Raw('$SMPROGRAMS'), chr(92) + s['name'] + '.lnk')}")
    L.append("!macroend")
    L.append("")

    L.append("!macro APP_RUN_VALUE")
    L.append(f"  StrCpy $0 {q(*exe_cmd(c['exe'], c['autostart_args']))}")
    L.append("!macroend")
    L.append("")

    L.append("!macro APP_LANGSTRINGS")
    for v in c["verbs"]:
        name = "APP_VERB_" + v["id"].replace("-", "_").replace(".", "_")
        for nsis, key in LANGS:
            L.append(f"  LangString {name} ${{LANG_{nsis}}} {q(v['label'][key])}")
    L.append("!macroend")
    return "\n".join(L) + "\n"


def setup_name(c: dict) -> str:
    return f"{c['artifact']}-{c['version']}-windows-x86_64-setup.exe"


def prepare(config: Path, payload: Path, stage: Path, root: Path | None, version: str | None, out: Path) -> str:
    c = load(config, root, version)
    stage.mkdir(parents=True, exist_ok=True)
    if any(stage.iterdir()):
        raise ConfigError(f"stage folder must be empty: {stage}")
    pdir = stage / "payload"
    stage_payload(payload, pdir, c["exe"])
    for src, dest in c["extra"]:
        target = pdir.joinpath(*safe_rel(dest).parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, target)
    rels = payload_files(pdir, c["include"], c["exclude"])
    files = [(pdir.joinpath(*PurePosixPath(r).parts), r) for r in rels]
    lic = stage / "LICENSE.txt"
    write_license(c["licenses"], lic)
    name = setup_name(c)
    (stage / "app.nsh").write_text(render(c, files, lic, (out / name).resolve()), encoding="utf-8")
    return name


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--config", required=True, type=Path)
    ap.add_argument("--payload", required=True, type=Path)
    ap.add_argument("--stage", required=True, type=Path)
    ap.add_argument("--root", type=Path)
    ap.add_argument("--version")
    ap.add_argument("--out", type=Path, default=Path("."))
    a = ap.parse_args(argv)
    try:
        print(prepare(a.config, a.payload, a.stage, a.root, a.version, a.out))
    except (ConfigError, OSError, zipfile.BadZipFile) as e:
        print(f"render.py: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
