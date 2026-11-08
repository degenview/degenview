# Flatpak

Every release carries `DegenView-<tag>-linux-x64.flatpak` and `DegenView-<tag>-linux-arm64.flatpak` next to the Linux tarballs. The app id is
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
| `.github/workflows/flatpak-check.yml` | Calls it on pull requests that touch the packaging, weekly on `main`, and on demand; not on `Cargo.toml`/`Cargo.lock` changes (see below) |

The workflow runs in two jobs. `prepare` runs on the plain runner: it generates
`cargo-sources.json` from `Cargo.lock` with a pinned `flatpak-cargo-generator.py` (a Flatpak build
has no network, so every crate is listed with its checksum), stamps the build id and today's release
into copies of the manifest and metainfo, and lints them with `desktop-file-validate` and
`appstreamcli validate --pedantic --no-net`. `build` runs `flatpak-builder` in the Flathub
`freedesktop-26.08` container and bundles the result, once per CPU in a matrix (pull requests build x86_64 only, `arm64: false`; a release builds both): x86_64 on
`ubuntu-latest` and aarch64 natively on GitHub's free `ubuntu-24.04-arm` runner (the container
image is multi-arch; no emulation). `cargo-sources.json` is the same for both and is never committed.

The check is not triggered by dependency changes: the build has no cache and takes 10-20 minutes,
and a crate that builds on the host nearly always builds in the sandbox. The weekly run on `main`
(Mondays) and the release build catch the rare one that does not. Dispatch `Flatpak check` by hand
on a pull request that adds a crate with native code (a `-sys` crate, `bindgen`, a C library).

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

A copy installed from a downloaded bundle has no remote, so `flatpak update` finds nothing for it;
one installed from the hosted repo below does. The app cannot tell which it is: Flatpak records
nothing about the remote in `/.flatpak-info` (upstream `flatpak-run.c` writes the instance and app
paths, the commits, `branch`, `arch` and the version, and no origin), and an earlier version of this
code that read an `origin` key never matched a real install. So the in-app check stays on and stays
non-installable: `self_update::install_target` returns `sandbox::FLATPAK_UPDATE_NOTE` when
`sandbox::is_flatpak()` (`FLATPAK_ID` or `/.flatpak-info`), which names both ways to update: run
`flatpak update`, or download the new bundle from the release page. The repo trails GitHub by up to a
few hours, which is why the note says "if". The Linux tarball is the form that updates itself
(`docs/agent-guides/releases-updates.md`, "Linux").

A check that knows the origin would use the Update Monitor portal
(`org.freedesktop.portal.Flatpak.CreateUpdateMonitor`), which reports updates from the app's own
remote to a sandboxed app. **Not built, not verified.**

## Hosted repo

Releases build one Flatpak **branch per update channel**: `stable` for a final release and `beta`
for a release candidate, a tag with `-rc.` (the form the Release Candidate channel offers). An
`-alpha.N` or `-beta.N` tag is offered to no channel by the updater (`update_check::Version::kind`,
`tools/release.sh` says the same), so its bundle is built on `dev`, which the repo never publishes.
Note this is narrower than `--prerelease`, which is any hyphen. `release.yml` passes the branch to
`flatpak.yml`'s `branch` input, which the pinned `flatpak-builder` action uses for both
`--default-branch` and the bundle's ref, so a bundle is `app/es.nico.degenview/<arch>/<stable|beta|dev>`.
The pull request check keeps `master`. Bundles from before this change are on `master` and are
not published.

`flatpak update` follows an installed app's own branch and never moves it to another, and the app's
Release Candidate channel accepts final releases. So the repo serves `beta` from the **newest release
of any kind, by version**: a final release is copied onto `beta` as well as `stable`, and an installed
`//beta` follows it from release candidates to finals and back to the next candidate. `stable` is the
newest final.

The repo itself lives in `degenview/flatpak` and is served at `https://flatpak.degenview.app`.
It pulls: a scheduled and manual workflow lists this repo's releases, picks the newest final (for
`stable`) and the newest release overall (for `beta`), verifies every bundle (`gh release verify-asset`, `gh attestation verify --repo
degenview/degenview`), imports them into an OSTree repo, signs it with a GPG key that only that
repo's `publish` environment holds, and deploys it to Pages. Nothing in this repo holds a token for
it. Users add it with:

```bash
flatpak remote-add --if-not-exists degenview https://flatpak.degenview.app/degenview.flatpakrepo
flatpak install degenview es.nico.degenview//stable   # or //beta
```

The runtime still comes from Flathub, so it must be a configured remote (or the `.flatpakref`, which
names it, is used). **Not verified:** none of this has run on Linux: the published repo, copying a final
onto `beta` with `flatpak build-commit-from`, an install and a `flatpak update` from it.

## Secret storage

API keys (Alpaca, CoinMarketCap, AI providers), the snapshot token and webhook secrets all go
through `keychain_webhook_secrets::platform`. On Linux that is `service::secret_service`, built on
the `oo7` crate: the desktop's Secret Service (GNOME Keyring, KWallet) outside a sandbox, and inside
a Flatpak an app-private keyring file opened with a key from the Secret portal (no extra
permission). `oo7` is async and this crate keeps async runtimes in `src/mcp/`, so the calls run on
one worker thread with a private current-thread runtime.

- The keyring opens on the first request; a failure to open (no Secret Service running) is
  remembered for 30 seconds, so a machine without one is asked once, not every frame.
- `exists` is answered from a cache that the app's own writes keep current: the webhook list asks
  once per row per frame. A secret changed by another program is not noticed until restart.
- `exists` and `delete` wait 10 seconds; `read` and `write` wait two minutes, because a locked
  keyring prompts the user to unlock it.
- `Scope::shared_group` is a Keychain access group and means nothing here: an item is its service
  plus its account.
- `DEGENVIEW_NO_KEYCHAIN` keeps secrets in memory, as on macOS.

## Windows secret storage

On Windows the same `platform::{read, exists, write, delete}` is `service::windows_credentials`: one
Credential Manager generic credential per item (`CredWriteW`, `CredReadW`, `CredDeleteW`, through
`windows-sys`, which was already in the tree), target name `<service>/<account>`, persisted with
`CRED_PERSIST_LOCAL_MACHINE`. Credential Manager encrypts the blob with the signed-in user's DPAPI
master key. It is stored local-machine, so Credential Manager does not roam it; the DPAPI key
behind it normally decrypts only for that user on that computer, with Microsoft's documented
exception that a roaming profile lets the same user decrypt on another computer. Items appear in Control Panel >
Credential Manager > Windows Credentials, where the user can remove them.

- The blob is the secret's UTF-8 bytes. A credential holds at most 2560 bytes, so a secret above
  that is refused with an error, never truncated. Every key and token fits; a webhook secret of
  2048 non-ASCII characters can exceed it.
- There is no unlock prompt and no per-application ACL: any process running as the same user can
  read the items, as on the macOS Keychain without an access group. What it keeps is secrets out of
  the SQLite file, its backups and iCloud Sync.
- Credential Manager never prompts, so there is no worker thread. `exists` is cached for 30 seconds
  (the webhook list asks once per row per frame) and kept current by the app's own writes.
- `Scope::shared_group` means nothing here, `DEGENVIEW_NO_KEYCHAIN` keeps secrets in memory, and
  temporary buffers that held a secret are overwritten before they are freed (best effort).
- The MCP remote-access TLS key (`mcp-tls.pem`) is not an item here: it is a certificate and key in
  one file, so on Windows the whole file is encrypted with DPAPI for the signed-in user
  (`mcp::tls_protect`, `CryptProtectData`, current-user scope, no UI). Windows normally lets only
  that user, on that computer, decrypt it, so a copy, a backup or a disk image is not readable
  without the user's Windows credentials. That is the usual case, not a guarantee: a roaming profile
  lets the same user decrypt on another computer. Any process of the same user can decrypt it (the
  fixed entropy string is not a secret). There is no plaintext form to migrate from, so a file that
  is not DPAPI-wrapped is refused and left alone. Unix keeps a `0600` file.
- **Not verified:** the real Credential Manager round trip (`credential_manager_round_trip`) runs on
  the `test-windows` CI job only (as do the `tls_protect` DPAPI tests). Nobody has saved a key in a
  running Windows build, restarted, and looked at the entry in Credential Manager, or turned on MCP
  remote access there and kept the same fingerprint across a restart. The Win32 calls were type-checked for
  `x86_64-pc-windows-gnu`, not built for MSVC here.

## File dialogs

`service::file_dialog` (CSV and image import, portfolio export, the watchlist export, snapshot "save
as") uses the XDG file chooser portal on Linux, through `rfd` without its GTK backend: `rfd` speaks to
the portal itself and loads `libdbus` when a dialog opens (the Freedesktop runtime has it), falling
back to `zenity` outside a sandbox when no portal runs. That is how a sandboxed app with no
filesystem permission reads or writes a file the user picks: the portal hands back a path under
`/run/user/<uid>/doc/` that the app may use. `rfd` cannot tell a cancelled dialog from one that
could not open, so both read as "chose nothing" and no error is shown.

`platform::has_file_dialogs()` is the one place that says which platforms have a panel; without
one, snapshots go to the Downloads folder under a free name and the file is revealed. "Show in File
Manager" still runs `xdg-open` on the folder, which a Flatpak routes through the OpenURI portal; it
may do nothing for a path the host cannot see.

## Not verified

Written without a Linux machine; CI builds the bundle but nothing has run it.

- The window opening, the charts rendering (wgpu or GL through `--device=dri`) and market data
  loading on Wayland and X11.
- That `flatpak-builder` accepts the manifest, including `skip` on the `dir` source and the
  `CARGO_HOME` path the generator's `cargo/config` expects. The `flatpak-check` job is the first
  test of this.
- That `appstreamcli validate --pedantic` passes with no warnings (the screenshot is 5344 × 3044,
  which Flathub may want smaller).
- Secret storage inside the sandbox (below): the worker and its caches are tested with a fake
  keyring, but nothing has talked to a real Secret Service or portal.
- File dialogs: that `rfd` finds `libdbus` and the portal in the runtime, that the chosen path is
  readable and writable, and that "Show in File Manager" works from inside the sandbox.

## Follow-ups

- The clipboard picture (`native_bridge::clipboard_picture`) is macOS-only, so "paste an image" has
  no Linux path yet.
- Flathub: a separate `flathub/es.nico.degenview` repository whose manifest builds a signed tag.
  Needs the id's domain verified and metainfo that lists each release.
