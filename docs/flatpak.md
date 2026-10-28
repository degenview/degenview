# Flatpak

Every release carries `DegenView-<tag>-linux-x64.flatpak` next to the Linux tarball. The app id is
`es.nico.degenview`, the same as the window's `app_id` in `src/lib.rs`; it is permanent once
published, so do not rename it.

## What is built

| File | Role |
| --- | --- |
| `packaging/flatpak/es.nico.degenview.yml` | The manifest: Freedesktop runtime and SDK `26.08`, the `rust-stable` SDK extension, an offline `cargo build --release --locked --bin degenview` |
| `packaging/flatpak/es.nico.degenview.desktop` | Launcher (the tarball keeps `packaging/degenview.desktop`) |
| `packaging/flatpak/es.nico.degenview.metainfo.xml` | AppStream metadata; the `0.0.0` release is a placeholder |
| `.github/workflows/flatpak.yml` | Reusable workflow that produces the bundle |
| `.github/workflows/release.yml` | Calls it for a tag; the `release` job waits for it, so the bundle is attached before the release is published (releases are immutable) |
| `.github/workflows/flatpak-check.yml` | Calls it on pull requests that touch the packaging, `Cargo.lock` or `Cargo.toml` |

The workflow runs in two jobs. `prepare` runs on the plain runner: it generates
`cargo-sources.json` from `Cargo.lock` with a pinned `flatpak-cargo-generator.py` (a Flatpak build
has no network, so every crate is listed with its checksum), stamps the build id and today's release
into copies of the manifest and metainfo, and lints them with `desktop-file-validate` and
`appstreamcli validate --pedantic --no-net`. `build` runs `flatpak-builder` in the Flathub
`freedesktop-26.08` container and bundles the result. `cargo-sources.json` is never committed.

Two values must move together when the runtime changes: `runtime-version` in the manifest and the
container tag in `flatpak.yml`. The `rust-stable` extension must ship rustc at least as new as
`rust-version` in `Cargo.toml` (checked against the 25.08 and 26.08 branches when this was written:
1.98).

## Sandbox

`--share=network`, `--share=ipc`, `--socket=wayland`, `--socket=fallback-x11` and `--device=dri`.
No filesystem access. The data and cache directories follow `XDG_DATA_HOME` and `XDG_CACHE_HOME`
(`store::data_dir`, `cache_dir`), which Flatpak points at the app's own folders. The MCP server
listens on `127.0.0.1`; `--share=network` uses the host network, so clients on the host reach it.
`degenview-alert-agent` is the macOS login item and is not installed.

## Updates

A downloaded bundle has no remote, so `flatpak update` finds nothing. The in-app update check
therefore stays on and stays non-installable: `self_update::install_target` returns a message
that names the release page when `sandbox::is_flatpak()` (`FLATPAK_ID` or `/.flatpak-info`). If
the app is ever hosted on Flathub, hide the check there instead.

## Not verified

Written without a Linux machine; CI builds the bundle but nothing has run it.

- The window opening, the charts rendering (wgpu or GL through `--device=dri`) and market data
  loading on Wayland and X11.
- That `flatpak-builder` accepts the manifest, including `skip` on the `dir` source and the
  `CARGO_HOME` path the generator's `cargo/config` expects. The `flatpak-check` job is the first
  test of this.
- That `appstreamcli validate --pedantic` passes with no warnings (the screenshot is 5344 × 3044,
  which Flathub may want smaller).
- Secret storage and file dialogs inside the sandbox: not part of this change, see the follow-ups.

## Follow-ups

- API keys and webhook secrets cannot be saved on Linux (no Keychain; `keychain_*` return "no
  Keychain on this platform"). A Secret Service backend through the Secret portal is the fix.
- File dialogs are AppleScript (`service::file_dialog`), so CSV and image import and snapshot "save
  as" do nothing on Linux. The portal file chooser is the fix.
- Flathub: a separate `flathub/es.nico.degenview` repository whose manifest builds a signed tag.
  Needs the id's domain verified and metainfo that lists each release.
