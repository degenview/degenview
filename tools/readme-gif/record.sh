#!/usr/bin/env bash
# Record the README hero GIF from the real app. macOS only, see README.md in this folder.
#
# usage: tools/readme-gif/record.sh [out.gif]       (default: ./hero.gif)
# env:   DEGENVIEW_BIN  a release build made with `--features dev-unsealed-mcp`
#                       (default: target-gif/release/degenview)
#        KEEP=1         keep the work directory (frames, contact sheet) after the run
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
bin=${DEGENVIEW_BIN:-$root/target-gif/release/degenview}
[ -x "$bin" ] || { echo "no binary at $bin, build one first (see README.md)" >&2; exit 1; }

work=$(mktemp -d)
out=$(cd "$(dirname "${1:-hero.gif}")" && pwd)/$(basename "${1:-hero.gif}")
export DEGENVIEW_RECORD_PORT=8791
DEGENVIEW_RECORD_TOKEN="dvmcp_$(openssl rand -hex 32)"
export DEGENVIEW_RECORD_TOKEN
home="$work/home"
mkdir -p "$home" "$work/cache" "$work/bin" "$work/take"

app=""
cleanup() {
    [ -n "$app" ] && kill "$app" 2>/dev/null || true
    if [ -z "${KEEP:-}" ]; then rm -rf "$work"; else echo "kept $work"; fi
}
trap cleanup EXIT

# A throwaway HOME and no Keychain or notifications: the real profile is never touched.
launch() {
    HOME="$home" DEGENVIEW_NO_KEYCHAIN=1 DEGENVIEW_NO_NOTIFICATIONS=1 DEGENVIEW_CACHE_DIR="$work/cache" \
        "$bin" >"$work/app.log" 2>&1 &
    app=$!
}

swiftc -O "$here/winlist.swift" -o "$work/bin/winlist"
swiftc -O "$here/mouse.swift" -o "$work/bin/mouse"

# The app creates its tables on first start, so run it once, quit, seed, and run it again.
launch; sleep 6; kill "$app"; wait "$app" 2>/dev/null || true
python3 -I "$here/seed.py" "$home"
launch; sleep 9

osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $app) to true"
sleep 1.5
read -r wid x y w h < <("$work/bin/winlist" "$app")
# Drag the bottom-right corner to 1400 x 1000 pt, then park the pointer clear of the window.
"$work/bin/mouse" "$app" drag $((x + w - 2)) $((y + h - 2)) $((x + 1398)) $((y + 998))
sleep 1
"$work/bin/mouse" "$app" park $((x + 2100)) $((y + 300))
read -r wid x y w h < <("$work/bin/winlist" "$app")
echo "window $wid, ${w}x${h} pt at $x,$y"

(cd "$here" && python3 drive.py "$app" "$wid" "$work/take")
python3 "$here/sheet.py" "$work/take" >/dev/null
python3 "$here/encode.py" "$work/take" "$out"
echo "$out"
