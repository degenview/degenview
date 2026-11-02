#!/bin/sh
# Builds the macOS disk image: DegenView.app next to an Applications shortcut, on a background that
# says to drag one onto the other.
#
#   tools/make-dmg.sh <path/to/DegenView.app> <out.dmg>
#
# Needs dmgbuild (`pip install dmgbuild==1.6.5`, in a venv on a Homebrew Python). It signs nothing:
# the release workflow signs and notarizes the image afterwards, around an app that is already
# signed, notarized and stapled. Works on the unsigned app `tools/bundle.sh` builds too.
set -eu

app="${1:?usage: make-dmg.sh <path/to/DegenView.app> <out.dmg>}"
out="${2:?usage: make-dmg.sh <path/to/DegenView.app> <out.dmg>}"

# Absolute paths first: the script moves to the repository root.
absolute() { printf '%s/%s' "$(cd "$(dirname "$1")" && pwd)" "$(basename "$1")"; }
app=$(absolute "$app")
mkdir -p "$(dirname "$out")"
out=$(absolute "$out")

cd "$(dirname "$0")/.."
root="$PWD"

if [ ! -d "$app" ] || [ "$(basename "$app")" != "DegenView.app" ]; then
    echo "error: $app is not a DegenView.app" >&2
    exit 1
fi
if ! command -v dmgbuild >/dev/null 2>&1; then
    echo "error: dmgbuild is not installed (pip install dmgbuild==1.6.5)" >&2
    exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# One TIFF holding the 1x and 2x art, so a Retina Mac shows the sharp one.
tiffutil -cathidpicheck packaging/macos/dmg-background.png packaging/macos/dmg-background@2x.png \
    -out "$work/background.tiff"

rm -f "$out"
dmgbuild -s packaging/macos/dmg-settings.py \
    -D "app=$app" -D "background=$work/background.tiff" -D "icon=$root/assets/brand/DegenView.icns" \
    DegenView "$out"
hdiutil verify "$out" >/dev/null
echo "Built $out"
