#!/usr/bin/env bash
# Builds the per-user Windows installer with Inno Setup around an already built degenview.exe.
#
#   tools/windows-installer.sh <tag> <path-to-degenview.exe> [x64|arm64]
#
# Writes dist/DegenView-<tag>-windows-<arch>-setup.exe (arch defaults to x64: the CPU the exe is built
# for, which decides what Windows the installer accepts). Runs in Git Bash on a Windows runner with Inno
# Setup installed (`choco install innosetup`); the release workflow and the installer check share it
# so they cannot drift.
set -euo pipefail

usage="usage: windows-installer.sh <tag> <path-to-degenview.exe> [x64|arm64]"
tag="${1:?$usage}"
exe="${2:?$usage}"
arch="${3:-x64}"
case "$arch" in x64|arm64) ;; *) echo "error: arch must be x64 or arm64, not $arch" >&2; exit 1 ;; esac
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
out="dist/DegenView-${tag}-windows-${arch}-setup.exe"
rm -f "$out"
"$iscc" /Q "/DAppVersion=${version}" "/DTag=${tag}" "/DArch=${arch}" \
  "/DExePath=$(cygpath -w "$(realpath "$exe")")" \
  "$(cygpath -w packaging/windows/degenview.iss)"

[ -s "$out" ] || { echo "error: $out was not produced" >&2; exit 1; }
echo "Built $out"
