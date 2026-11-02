#!/usr/bin/env bash
# Builds the per-user Windows installer with Inno Setup around an already built degenview.exe.
#
#   tools/windows-installer.sh <tag> <path-to-degenview.exe>
#
# Writes dist/DegenView-<tag>-windows-x64-setup.exe. Runs in Git Bash on a Windows runner with Inno
# Setup installed (`choco install innosetup`); the release workflow and the installer check share it
# so they cannot drift.
set -euo pipefail

tag="${1:?usage: windows-installer.sh <tag> <path-to-degenview.exe>}"
exe="${2:?usage: windows-installer.sh <tag> <path-to-degenview.exe>}"
version="${tag#v}"

cd "$(dirname "$0")/.."
[ -f "$exe" ] || { echo "error: $exe does not exist" >&2; exit 1; }

iscc=""
for candidate in \
  "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
  "/c/Program Files/Inno Setup 6/ISCC.exe" \
  "${LOCALAPPDATA:-}/Programs/Inno Setup 6/ISCC.exe"; do
  if [ -x "$candidate" ]; then iscc="$candidate"; break; fi
done
[ -n "$iscc" ] || iscc="$(command -v ISCC.exe || true)"
[ -n "$iscc" ] || { echo "error: Inno Setup (ISCC.exe) is not installed" >&2; exit 1; }

# Git Bash would rewrite the /D switches as paths.
export MSYS2_ARG_CONV_EXCL='*'
rm -f "dist/DegenView-${tag}-windows-x64-setup.exe"
"$iscc" /Q "/DAppVersion=${version}" "/DTag=${tag}" "/DExePath=$(cygpath -w "$(realpath "$exe")")" \
  "$(cygpath -w packaging/windows/degenview.iss)"

out="dist/DegenView-${tag}-windows-x64-setup.exe"
[ -s "$out" ] || { echo "error: $out was not produced" >&2; exit 1; }
echo "Built $out"
