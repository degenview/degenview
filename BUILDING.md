# Building DegenView

How to build DegenView from source, package it and cut a release. To just use the app, see the
[download section](README.md#download) of the README.

## Requirements

- macOS (Apple Silicon or Intel), Windows (x64 or ARM64), or Linux (x64 or arm64) with glibc 2.35 or
  newer (Ubuntu 22.04, Debian 12, Fedora 36 and later). On an older distro such as RHEL 9, use the
  Flatpak
- [Rust](https://rustup.rs) 1.95 or newer (`cargo` on your `PATH`)
- A free Alpaca account and API keys only if you want stock data. Set `DEGENVIEW_ALPACA_KEY_ID`
  and `DEGENVIEW_ALPACA_SECRET_KEY`, or store them in Keychain on macOS (the Secret Service on Linux,
  Credential Manager on Windows)
- No CoinMarketCap API key is required. Adding one in Settings is optional
  (`DEGENVIEW_CMC_API_KEY`)

## Build and run

```bash
cargo run
```

Build a release binary:

```bash
cargo build --release
```

The binary lands at `target/release/degenview` (`degenview.exe` on Windows).

Run the tests with `cargo test`. The Pine corpus test is slow, so it sits behind a feature:
`cargo test --features corpus`.

## macOS app bundle

Build a Finder-launchable app bundle:

```bash
./tools/bundle.sh
open target/DegenView.app
```

The default bundle is unsigned and uses foreground-only alert evaluation. To make the embedded
`DegenViewAlertAgent.app` available through Login Items, sign both bundles with an Apple-issued
identity, an explicit shared Keychain access group, and one provisioning profile per bundle. The
profiles (Developer ID, for the App IDs `es.nico.degenview` and `es.nico.degenview.alert-agent`)
are required. `keychain-access-groups` is a restricted entitlement, and macOS kills a signed
build at launch without a profile that grants it.

```bash
DEGENVIEW_CODESIGN_IDENTITY="Developer ID Application: Example (TEAMID)" \
DEGENVIEW_KEYCHAIN_ACCESS_GROUP="TEAMID.es.nico.degenview.shared" \
DEGENVIEW_APP_PROFILE=/path/to/DegenView.provisionprofile \
DEGENVIEW_HELPER_PROFILE=/path/to/DegenViewAlertAgent.provisionprofile \
./tools/bundle.sh
```

The helper is installed at
`DegenView.app/Contents/Library/LoginItems/DegenViewAlertAgent.app`. DegenView falls back to
foreground evaluation when registration is unavailable, awaiting approval, or its heartbeat is
stale.

### iCloud Sync

iCloud Sync is opt-in at build time. `DEGENVIEW_ICLOUD=1 ./tools/bundle.sh` compiles the CloudKit
bridge (macOS with the Xcode command line tools) and signs the app with iCloud entitlements, which
needs the App ID, container and provisioning profile described in
[docs/icloud-sync.md](docs/icloud-sync.md). Run `tools/verify-icloud.sh target/DegenView.app` on the
result. Without the variable nothing changes.

## Linux

The release tarball ships a `degenview.desktop` launcher and `degenview.png` icon. To get a launcher
and dock icon, install them:

```bash
install -Dm755 degenview "$HOME/.local/bin/degenview"
install -Dm644 degenview.png "$HOME/.local/share/icons/hicolor/512x512/apps/degenview.png"
install -Dm644 degenview.desktop "$HOME/.local/share/applications/degenview.desktop"
```

Installed in `~/.local/bin` (or any folder you own), the binary updates itself from inside the app. It
downloads the new tarball, replaces `degenview` and restarts. A binary in a folder you cannot write to,
such as `/usr/local/bin` or one a package manager owns, only shows a link to the release page. The
launcher and icon files above are not touched by an update.

Each release also carries a Flatpak bundle, `DegenView-<tag>-linux-x64.flatpak` or
`-linux-arm64.flatpak` (the tarball likewise has an `-arm64` build), which needs no
manual install steps and runs sandboxed (network, display and GPU only):

```bash
flatpak install --user DegenView-<tag>-linux-<x64|arm64>.flatpak
flatpak run es.nico.degenview
```

A bundle has no remote, so `flatpak update` does not find newer versions and the app cannot replace
itself in the sandbox. Download the next one from the release page. How it is built and what is
unverified: [docs/flatpak.md](docs/flatpak.md).

## Windows

`DegenView-<tag>-windows-x64-setup.exe` (or `-windows-arm64-setup.exe` on a Windows on ARM PC such as
a Snapdragon X laptop, where the x64 build only runs emulated) installs for the current user, with no
admin rights, into `%LOCALAPPDATA%\Programs\DegenView`. It adds a Start menu entry and an uninstaller,
and keeps your data when removed. The `.zip` is the portable alternative: unzip it into a folder you
own. Both update themselves from inside the app. Neither is signed yet, so SmartScreen may ask you to
approve the first launch.

## Releases

Push a tag like `v0.1.0` and the Release workflow builds macOS (arm64 and Intel, as a disk image and a
zip), Windows (x64 and ARM64, each an installer and a portable zip), and Linux binaries (a tarball and a
Flatpak), then publishes a GitHub release you can download.

The Release workflow signs the macOS builds the same way as above, then notarizes and staples them. It
reads these repository secrets: `MACOS_CERT_P12` (base64 `.p12` of the Developer ID Application
certificate and key), `MACOS_CERT_PASSWORD`, `MACOS_CODESIGN_IDENTITY`,
`MACOS_KEYCHAIN_ACCESS_GROUP` (`TEAMID.es.nico.degenview.shared`), `MACOS_PROFILE_APP` and
`MACOS_PROFILE_HELPER` (base64 provisioning profiles), and an App Store Connect API key
for `notarytool`: `ASC_KEY_P8` (base64 `.p8`), `ASC_KEY_ID`, `ASC_ISSUER_ID`.

On macOS the disk image holds the signed and notarized app. The app updates itself from inside the
app by downloading the `.zip` with the same app in it, so it must stay in Applications and not on the
disk image.

## More

- [Architecture](docs/architecture.md): project structure and data flow, including the
  [MCP server](docs/architecture/mcp.md)
- [AGENTS.md](AGENTS.md): conventions for AI coding agents working in this repository
