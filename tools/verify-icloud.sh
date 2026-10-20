#!/bin/sh
# Checks a signed DegenView.app is correctly set up for iCloud Sync, before it is notarized or
# shipped. Usage: tools/verify-icloud.sh target/DegenView.app [Production|Development]
set -eu

app="${1:?usage: verify-icloud.sh DegenView.app [Production|Development]}"
expected="${2:-Production}"
helper="$app/Contents/Library/LoginItems/DegenViewAlertAgent.app"
failed=0

fail() {
    echo "verify-icloud: $1" >&2
    failed=1
}

entitlements() {
    codesign -d --entitlements :- "$1" 2>/dev/null
}

app_ents=$(entitlements "$app") || fail "the app has no entitlements (is it signed?)"
echo "$app_ents" | grep -q "com.apple.developer.icloud-container-identifiers" \
    || fail "the app lacks the iCloud container entitlement"
echo "$app_ents" | grep -q "iCloud.es.nico.degenview" \
    || fail "the app's container is not iCloud.es.nico.degenview"
echo "$app_ents" | grep -q "<string>CloudKit</string>" \
    || fail "the app lacks the CloudKit service entitlement"
echo "$app_ents" | tr -d '\n ' | grep -q "icloud-container-environment</key><string>$expected</string>" \
    || fail "the app's iCloud environment is not $expected"

# A release must be Production: a Development build would write people's data to test storage.
if [ "$expected" != "Production" ]; then
    echo "verify-icloud: note: this build uses the $expected environment (not for release)" >&2
fi

if [ -d "$helper" ]; then
    entitlements "$helper" | grep -qi "icloud" \
        && fail "the login item must not carry iCloud entitlements"
fi

# The profile embedded in the app must grant what the entitlements claim.
profile="$app/Contents/embedded.provisionprofile"
if [ -f "$profile" ]; then
    decoded=$(security cms -D -i "$profile" 2>/dev/null || true)
    echo "$decoded" | grep -q "iCloud.es.nico.degenview" \
        || fail "the embedded provisioning profile does not grant the iCloud container"
    echo "$decoded" | grep -q "CloudKit" \
        || fail "the embedded provisioning profile does not grant CloudKit"
else
    fail "the app has no embedded provisioning profile"
fi

# The Swift runtime comes from macOS (/usr/lib/swift); nothing Swift may be bundled, and no
# machine-specific path (an Xcode toolchain) may be baked into the binary. The update check only
# accepts the known executables.
rpaths=$(otool -l "$app/Contents/MacOS/DegenView" | awk '/LC_RPATH/{getline; getline; print $2}')
for path in $rpaths; do
    [ "$path" = "/usr/lib/swift" ] || fail "the app has an unexpected run-path: $path"
done
if find "$app" -name "*.dylib" -o -name "*.framework" | grep -q .; then
    fail "the app bundles a library; the update check only accepts the two known executables"
fi
if [ -d "$helper" ] && otool -L "$helper/Contents/MacOS/DegenViewAlertAgent" | grep -q "libswift"; then
    fail "the login item must not link Swift"
fi

[ "$failed" -eq 0 ] && echo "verify-icloud: $app is set up for iCloud ($expected)"
exit "$failed"
