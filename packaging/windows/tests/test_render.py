"""app.toml -> app.nsh rendering, payload staging, and (when makensis is installed) a full build.

    python3.11 -I -m unittest discover -s packaging/windows/tests -v
"""

import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
TEMPLATE = HERE.parent
sys.path.insert(0, str(TEMPLATE))
import render  # noqa: E402


def tiny_ico(path: Path) -> None:
    """A valid 1x1 32-bit icon (makensis parses the icon it embeds)."""
    bmp = struct.pack("<IiiHHIIiiII", 40, 1, 2, 1, 32, 0, 8, 0, 0, 0, 0) + b"\x00\x00\xff\xff" + b"\x00" * 4
    path.write_bytes(struct.pack("<HHH", 0, 1, 1) + struct.pack("<BBBBHHII", 1, 1, 0, 0, 1, 32, len(bmp), 22) + bmp)


class Repo:
    """A fake app repo: packaging/windows/app.toml, Cargo.toml, icon, licenses, a release zip."""

    def __init__(self, toml: str, files=None, cargo='[workspace.package]\nversion = "1.4.2"\n'):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "packaging/windows").mkdir(parents=True)
        (self.root / "assets").mkdir()
        tiny_ico(self.root / "assets/app.ico")
        (self.root / "LICENSE-MIT").write_text("MIT text\n")
        (self.root / "LICENSE-APACHE").write_text("Apache text\n")
        (self.root / "Cargo.toml").write_text(cargo)
        self.config = self.root / "packaging/windows/app.toml"
        self.config.write_text(toml)
        self.zip = self.root / "dist/demo-windows-x86_64.zip"
        self.zip.parent.mkdir()
        with zipfile.ZipFile(self.zip, "w") as z:
            for name, data in (files or {"demo.exe": b"MZ fake", "README.md": b"# demo", "x.reg": b"reg"}).items():
                z.writestr("demo-windows-x86_64/" + name, data)

    def prepare(self, **kw):
        stage = self.root / "stage"
        out = self.root / "out"
        name = render.prepare(self.config, kw.pop("payload", self.zip), stage, kw.pop("root", None), kw.pop("version", None), out)
        return name, (stage / "app.nsh").read_text(encoding="utf-8"), stage

    def close(self):
        self.tmp.cleanup()


BASE = """
[app]
id = "otherwise-demo"
artifact = "demo"
name = "Démo"
exe = "demo.exe"
version_from = "../../Cargo.toml"
icon = "../../assets/app.ico"
license = ["../../LICENSE-MIT", "../../LICENSE-APACHE"]
"""

FULL = BASE + """
[files]
exclude = ["*.reg"]

[shortcuts]
desktop_default = true

[[protocols]]
scheme = "otherwise-demo"
description = "Demo link"

[[file_associations]]
extensions = [".DEMO", ".dmo"]
prog_id = "Otherwise.Demo.1"
description = "Demo file"

[[context_menu]]
id = "AddToDemo"
targets = ["files", "folders", "folder_background"]
label = { en = "Add to demo", es = "Añadir a demo" }

[autostart]
enabled = true
args = "--background"
"""


class RenderTest(unittest.TestCase):
    def setUp(self):
        self.repos = []

    def tearDown(self):
        for r in self.repos:
            r.close()

    def repo(self, *a, **kw):
        r = Repo(*a, **kw)
        self.repos.append(r)
        return r

    def test_minimal_defaults(self):
        name, nsh, stage = self.repo(BASE).prepare()
        self.assertEqual(name, "demo-1.4.2-windows-x86_64-setup.exe")
        self.assertIn('!define APP_VERSION_QUAD "1.4.2.0"', nsh)
        self.assertIn('!define APP_PUBLISHER "Otherwise Technologies LLC"', nsh)
        self.assertIn('!define APP_URL "https://otherwisetech.com"', nsh)
        self.assertIn('!define APP_DESKTOP_DEFAULT 0', nsh)
        self.assertNotIn("APP_AUTOSTART", nsh.split("!macro")[0])
        # zip top folder stripped; everything included by default
        self.assertIn('File /oname=demo.exe', nsh)
        self.assertIn('File /oname=x.reg', nsh)
        self.assertNotIn("WriteRegStr", nsh)
        lic = (stage / "LICENSE.txt").read_bytes()
        self.assertTrue(lic.startswith(b"\xef\xbb\xbfMIT text\r\n"))
        self.assertIn(b"Apache text\r\n", lic)

    def test_full_registry(self):
        _, nsh, _ = self.repo(FULL).prepare(version="2.0.0-beta.1")
        self.assertIn('!define APP_VERSION "2.0.0-beta.1"', nsh)
        self.assertIn('!define APP_VERSION_QUAD "2.0.0.0"', nsh)
        self.assertNotIn("x.reg", nsh)
        cmd = '"$\\"$INSTDIR\\demo.exe$\\" $\\"%1$\\""'
        # protocol: same values as otherwise_link::register::windows_reg_commands
        self.assertIn('WriteRegStr HKCU "Software\\Classes\\otherwise-demo" "" "URL:Demo link"', nsh)
        self.assertIn('WriteRegStr HKCU "Software\\Classes\\otherwise-demo" "URL Protocol" ""', nsh)
        self.assertIn('WriteRegStr HKCU "Software\\Classes\\otherwise-demo\\DefaultIcon" "" "$\\"$INSTDIR\\demo.exe$\\",0"', nsh)
        self.assertIn('WriteRegStr HKCU "Software\\Classes\\otherwise-demo\\shell\\open\\command" "" ' + cmd, nsh)
        # associations, lower-cased extensions
        self.assertIn('WriteRegStr HKCU "Software\\Classes\\.demo\\OpenWithProgids" "Otherwise.Demo.1" ""', nsh)
        self.assertIn('!insertmacro OW_CLAIM_DEFAULT "Software\\Classes\\.dmo" "Otherwise.Demo.1"', nsh)
        self.assertIn('!insertmacro OW_RELEASE_DEFAULT "Software\\Classes\\.dmo" "Otherwise.Demo.1"', nsh)
        # context menu on files, folders and the folder background (%V there)
        self.assertIn('WriteRegStr HKCU "Software\\Classes\\*\\shell\\AddToDemo\\command" "" ' + cmd, nsh)
        self.assertIn('WriteRegStr HKCU "Software\\Classes\\Directory\\shell\\AddToDemo" "" "$(APP_VERB_AddToDemo)"', nsh)
        self.assertIn('"Software\\Classes\\Directory\\Background\\shell\\AddToDemo\\command" "" "$\\"$INSTDIR\\demo.exe$\\" $\\"%V$\\""', nsh)
        self.assertIn('LangString APP_VERB_AddToDemo ${LANG_SPANISH} "Añadir a demo"', nsh)
        self.assertIn('LangString APP_VERB_AddToDemo ${LANG_FRENCH} "Add to demo"', nsh)  # falls back to en
        # every key the installer writes is removed only when it still points here
        self.assertEqual(nsh.count("OW_DELETE_KEY_IF_OURS"), 1 + 1 + 3)
        self.assertIn('StrCpy $0 "$\\"$INSTDIR\\demo.exe$\\" --background"', nsh)
        self.assertIn("!define APP_AUTOSTART 1", nsh)
        self.assertIn("!define APP_AUTOSTART_DEFAULT 0", nsh)

    def test_uninstall_lists_only_installed_files(self):
        files = {"demo.exe": b"MZ", "data/a.txt": b"a", "data/deep/b.txt": b"b"}
        _, nsh, _ = self.repo(BASE, files=files).prepare()
        un = nsh.split("!macro APP_UNINSTALL_FILES")[1].split("!macroend")[0]
        self.assertEqual([line.strip() for line in un.strip().splitlines()], [
            'Delete "$INSTDIR\\data\\a.txt"',
            'Delete "$INSTDIR\\data\\deep\\b.txt"',
            'Delete "$INSTDIR\\demo.exe"',
            'Delete "$INSTDIR\\otherwise-demo.ico"',
            'RMDir "$INSTDIR\\data\\deep"',
            'RMDir "$INSTDIR\\data"',
        ])
        self.assertNotIn("RMDir /r", nsh)
        self.assertNotIn("RMDir /r", (TEMPLATE / "installer.nsi").read_text(encoding="utf-8"))
        self.assertIn('SetOutPath "$INSTDIR\\data\\deep"', nsh)

    def test_escaping(self):
        toml = BASE.replace('name = "Démo"', 'name = "Demo $HOME"') + '[[protocols]]\nscheme = "x-demo"\ndescription = "say \\"hi\\" $1"\n'
        _, nsh, _ = self.repo(toml).prepare()
        self.assertIn('!define APP_NAME "Demo $$HOME"', nsh)
        self.assertIn('"URL:say $\\"hi$\\" $$1"', nsh)

    def test_exe_payload_and_package_version(self):
        r = self.repo(BASE, cargo='[package]\nname = "demo"\nversion = "0.9.1"\n')
        exe = r.root / "target-demo-app.exe"
        exe.write_bytes(b"MZ")
        name, nsh, stage = r.prepare(payload=exe)
        self.assertEqual(name, "demo-0.9.1-windows-x86_64-setup.exe")
        self.assertTrue((stage / "payload/demo.exe").exists())

    def test_rejects(self):
        cases = {
            "bad scheme": FULL.replace('scheme = "otherwise-demo"', 'scheme = "Otherwise Demo"'),
            "bad ext": FULL.replace('".dmo"', '"dmo"'),
            "bad target": FULL.replace('"folder_background"', '"desktop"'),
            "missing exe": BASE.replace('exe = "demo.exe"', 'exe = "other.exe"'),
            "bad id": BASE.replace('id = "otherwise-demo"', 'id = "a\\\\b"'),
            "bad name": BASE.replace('name = "Démo"', 'name = "a/b"'),
            "no license": BASE.replace('license = ["../../LICENSE-MIT", "../../LICENSE-APACHE"]', ""),
            "no version": BASE.replace('version_from = "../../Cargo.toml"', ""),
            "bad version": BASE.replace('version_from = "../../Cargo.toml"', 'version = "1.2"'),
        }
        for what, toml in cases.items():
            with self.subTest(what):
                with self.assertRaises(render.ConfigError):
                    self.repo(toml).prepare()

    def test_zip_slip(self):
        r = self.repo(BASE)
        with zipfile.ZipFile(r.zip, "w") as z:
            z.writestr("demo.exe", b"MZ")
            z.writestr("../evil.txt", b"x")
        with self.assertRaises(render.ConfigError):
            r.prepare()
        self.assertFalse((r.root / "evil.txt").exists())

    def test_deterministic(self):
        r1, r2 = self.repo(FULL), self.repo(FULL)
        n1 = r1.prepare()[1].replace(str(r1.root), "R")
        n2 = r2.prepare()[1].replace(str(r2.root), "R")
        self.assertEqual(n1, n2)

    def test_examples_parse(self):
        for ex in sorted((TEMPLATE / "examples").glob("*/app.toml")):
            with self.subTest(ex.parent.name):
                c = render.load(ex, version="1.0.0")
                self.assertTrue(c["id"].startswith("otherwise-"))


@unittest.skipUnless(shutil.which("makensis"), "makensis not installed")
class BuildTest(unittest.TestCase):
    def build(self, repo: Repo, out: Path) -> Path:
        env = dict(os.environ, TMPDIR=str(repo.root))
        r = subprocess.run(["bash", str(TEMPLATE / "build-installer.sh"), "--config", str(repo.config),
                            "--payload", str(repo.zip), "--out", str(out)], capture_output=True, text=True, env=env)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        return Path(r.stdout.strip().splitlines()[-1])

    def test_build_full_twice_identical(self):
        a, b = Repo(FULL), Repo(FULL)
        try:
            ea, eb = self.build(a, a.root / "o"), self.build(b, b.root / "o")
            self.assertEqual(ea.name, "demo-1.4.2-windows-x86_64-setup.exe")
            data = ea.read_bytes()
            self.assertEqual(data[:2], b"MZ")
            self.assertIn(b"Nullsoft", data)
            digest = hashlib.sha256(data).hexdigest()
            self.assertEqual(digest, hashlib.sha256(eb.read_bytes()).hexdigest())
            self.assertEqual((ea.parent / (ea.name + ".sha256")).read_text().split(), [digest, ea.name])
        finally:
            a.close()
            b.close()


if __name__ == "__main__":
    unittest.main()
