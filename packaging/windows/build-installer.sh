#!/usr/bin/env bash
# Builds <artifact>-<version>-windows-x86_64-setup.exe (+ .sha256) from an app.toml and the
# app's Windows build (the release zip, the .exe, or a folder). Runs on macOS, Linux and
# Windows (Git Bash). Needs makensis 3.x and Python 3.11+.
#
#   build-installer.sh --config packaging/windows/app.toml --payload dist/app-windows-x86_64.zip \
#                      [--version 1.2.3] [--root DIR] [--out DIR] [--sign-cmd CMD] [--keep-stage]
#
# --version  defaults to app.version / app.version_from (Cargo.toml) in app.toml
# --root     base for relative paths in app.toml (default: the folder holding app.toml)
# --out      where the installer goes (default: current folder)
# --sign-cmd a command that signs the file given as its last argument, run on the uninstaller
#            and on the installer (e.g. "osslsigncode-wrapper.sh"); unsigned without it
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
config="" payload="" version="" root="" out="." keep=0 sign=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --config) config="$2"; shift 2 ;;
    --payload) payload="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    --root) root="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --sign-cmd) sign="$2"; shift 2 ;;
    --keep-stage) keep=1; shift ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "build-installer.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
[[ -n "$config" && -n "$payload" ]] || { echo "build-installer.sh: --config and --payload are required" >&2; exit 2; }

py=""
for c in "${PYTHON:-}" python3 python3.13 python3.12 python3.11 python; do
  [[ -n "$c" ]] && command -v "$c" >/dev/null 2>&1 && "$c" -c 'import tomllib' 2>/dev/null && { py="$c"; break; }
done
[[ -n "$py" ]] || { echo "build-installer.sh: needs Python 3.11+ (tomllib); set PYTHON=" >&2; exit 2; }
nsis="${MAKENSIS:-makensis}"
command -v "$nsis" >/dev/null 2>&1 || { echo "build-installer.sh: makensis not found (brew install makensis / apt-get install nsis)" >&2; exit 2; }

mkdir -p "$out"
out="$(cd "$out" && pwd)"
stage="$(mktemp -d "${TMPDIR:-/tmp}/ow-installer.XXXXXX")"
[[ $keep == 1 ]] || trap 'rm -rf "$stage"' EXIT

args=(--config "$config" --payload "$payload" --stage "$stage/s" --out "$out")
[[ -n "$version" ]] && args+=(--version "$version")
[[ -n "$root" ]] && args+=(--root "$root")
name="$("$py" -I "$here/render.py" "${args[@]}")"

# -WX: every warning fails the build. -INPUTCHARSET: the scripts are UTF-8 without a BOM.
defs=(-DAPP_NSH="$stage/s/app.nsh")
[[ -n "$sign" ]] && defs+=(-DSIGN_CMD="$sign")
"$nsis" -WX -V2 -INPUTCHARSET UTF8 "${defs[@]}" "$here/installer.nsi"

cd "$out"
if command -v sha256sum >/dev/null 2>&1; then sha256sum "$name" > "$name.sha256"; else shasum -a 256 "$name" > "$name.sha256"; fi
[[ $keep == 1 ]] && echo "stage: $stage/s" >&2
echo "$out/$name"
