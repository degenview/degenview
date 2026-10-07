#!/bin/bash
# Run-script build phase: gives every compile a new CFBundleVersion.
#
# The counter lives in `.build-number` at the repo root (gitignored, so builds never dirty the
# tree; each checkout counts on its own). It is written into the *built* Info.plist, not the
# project, and runs before code signing so the signature covers the new value. A fresh checkout
# starts from CURRENT_PROJECT_VERSION.
set -euo pipefail

# Indexing and other non-compiling actions are not builds.
[ "${ACTION:-build}" = "indexbuild" ] && exit 0

counter="${SRCROOT}/.build-number"
plist="${TARGET_BUILD_DIR}/${INFOPLIST_PATH}"

[ -f "$plist" ] || { echo "warning: bump-build-number: no Info.plist at $plist"; exit 0; }

current=$(tr -dc '0-9' < "$counter" 2>/dev/null || true)
[ -n "$current" ] || current="${CURRENT_PROJECT_VERSION:-0}"
next=$((current + 1))

printf '%s\n' "$next" > "$counter.tmp" && mv "$counter.tmp" "$counter"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $next" "$plist"
echo "Build number: $next"
