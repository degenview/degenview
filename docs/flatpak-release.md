# Flatpak for Linux releases — what it takes

## Context
Linux release today = `DegenView-<tag>-linux-x64.tar.gz` (binary + png + `packaging/degenview.desktop`), built in the `linux-x64` leg of `.github/workflows/release.yml`. User installs by hand into `~/.local` (README ~l.178). Self-update is macOS-only already (`src/service/self_update.rs::install_target`), so nothing writes to `/app`. Goal: ship via Flatpak (Flathub and/or `.flatpak` bundle on the GitHub release).

Findings below come from an Explore pass; not yet re-verified line by line.

## Good news (no work)
- Pure rustls, no openssl. `rusqlite` bundled. No git deps, `Cargo.lock` committed -> `flatpak-cargo-generator.py Cargo.lock` works offline.
- Data/cache dirs use `XDG_DATA_HOME`/`XDG_CACHE_HOME` (`src/store/mod.rs`) -> Flatpak remaps automatically.
- winit has wayland+x11 in one binary; wgpu -> Vulkan/GL via `--device=dri`.
- MCP server is loopback HTTP, off by default; works with `--share=network`.

## Work items
### A. Packaging (the Flatpak itself)
1. `packaging/flatpak/es.nico.degenview.yml`: runtime `org.freedesktop.Platform` (current branch), SDK + `org.freedesktop.Sdk.Extension.rust-stable` (must give rustc >= 1.95, `rust-version` in Cargo.toml). Offline cargo build with generated `cargo-sources.json`. Set `DEGENVIEW_BUILD_ID` in `build-options.env` (build.rs otherwise records `unknown` outside git).
2. Finish-args: `--share=network`, `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc`, `--device=dri`. D-Bus for accesskit/portals is covered by default session access to portals. Consider `--filesystem=xdg-download` only if the snapshot export stays path-based (see C).
3. Rename `degenview.desktop` -> `es.nico.degenview.desktop`, set `Icon=es.nico.degenview`, `StartupWMClass`. Install icon (prefer SVG; only PNG exists, 512 RGB).
4. AppStream `es.nico.degenview.metainfo.xml`: summary, description, GPL-3.0-only, screenshot URL (`assets/screenshots/screenshot.png`), `<release>` per version, OARS content rating.
5. App id: `es.nico.degenview` must match a domain you control for Flathub (nico.es) or switch to `io.github.degenview.*`. Decide before first publish — id is permanent.

### B. Code changes needed in-app
1. Suppress update nag under Flatpak: detect `FLATPAK_ID`/`/.flatpak-info`; in `src/host/update_host.rs` (~l.420 `available_phase`) and strings in `src/ui/settings_about.rs`, `update_indicator.rs` say "update via Flathub / `flatpak update`" or hide. (Self-update already refuses on Linux.)
2. Drop unused `libssl-dev` from workflows (cosmetic).

### C. Pre-existing Linux gaps the sandbox exposes (decide scope)
1. **No secret storage on Linux**: `keychain_api_keys.rs` / `keychain_webhook_secrets.rs` return "no Keychain on this platform". Saving API keys / webhook secrets in Settings fails. Needs Secret Service (libsecret / `oo7` crate, works via portal in Flatpak). Likely the biggest item.
2. **File dialogs are osascript-only** (`src/service/file_dialog.rs`): CSV import/export, image pick broken on Linux. Needs `rfd` with xdg-portal backend (works in sandbox, no broad filesystem access).
3. `snapshot_export.rs` writes to `$HOME/Downloads` — not portal-aware; route through the file dialog or grant `xdg-download`.
4. `reveal_in_file_manager` uses `xdg-open` on a path; limited in sandbox (OpenURI portal only for accessible paths).
5. Extra bin `degenview-alert-agent` is macOS-only but still built; skip installing it (or gate it) in the Flatpak.

### D. Release pipeline
- Option 1 (cheap): add a job in `release.yml` running `flatpak-builder` (container `ghcr.io/flathub-infra/flatpak-github-actions:freedesktop-<ver>`), produce `DegenView-<tag>-linux-x64.flatpak` via `flatpak build-bundle`, attach to the same `gh release create` (releases are immutable, so it must be built before publish). Optionally keep the tarball.
- Option 2 (Flathub): separate repo `flathub/es.nico.degenview` with manifest pointing at the signed tag + sha256; Flathub builds, signs, hosts, auto-updates users. Needs Flathub review (id ownership, metainfo, no unneeded permissions). Each release = bump tag/commit in that manifest (bot `flathub-infra` can auto-PR via `x-checker-data`).
- Recommendation: Option 2 for real users + Option 1 bundle as a fallback artifact; CI test job for `appstreamcli validate` and `desktop-file-validate`.
- Per AGENTS.md: touches release/packaging -> ship as MINOR RC first (`-rc.N`) if the updater or release.yml change; follow `docs/agent-guides/releases-updates.md`, update that doc and README install section.

## Suggested phasing
1. PR 1: manifest + desktop/metainfo/icon + FLATPAK_ID update-nag suppression; build/run locally with `flatpak-builder --user --install`.
2. PR 2: Secret Service keychain backend + rfd portal file dialogs (usable app).
3. PR 3: CI bundle job in `release.yml` + docs; then Flathub submission.

## Verification
- `flatpak-builder --user --install --force-clean build packaging/flatpak/es.nico.degenview.yml`, `flatpak run es.nico.degenview` on Wayland and X11 (Linux VM/box; not testable on this Mac).
- `appstreamcli validate --pedantic`, `desktop-file-validate`, `flatpak run --command=flatpak-builder-lint`.
- In sandbox: chart renders (GPU), market data loads, save API key persists across restart, CSV import dialog opens, update banner hidden, MCP reachable on localhost.
