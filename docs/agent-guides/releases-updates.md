# Releases and updates guidance

## Release procedure

`Cargo.toml` is the only app-version source (read by About, update checks, `tools/bundle.sh`).
Never edit the version or create a release tag by hand. Pick the version with
[Choosing the release type](../../AGENTS.md#choosing-the-release-type).

## Versions and channels

SemVer 2.0: `MAJOR.MINOR.PATCH` or `MAJOR.MINOR.PATCH-rc.N` (`tools/release.sh` also accepts
`-alpha.N`/`-beta.N`; no channel offers them). Pre-1.0: MINOR = feature or format change, PATCH =
fix. Always dotted `rc.N`, never `rc1` (numeric parts compare as numbers: `rc.10` after `rc.2`). The
final `0.6.0` is a new tag rebuilt from the last commit, never a promoted RC binary.

```
0.5.1  <  0.6.0-rc.1  <  0.6.0-rc.2  <  0.6.0-rc.10  <  0.6.0
```

- Channels: chosen in Settings ▸ About, saved per Mac in local-only `update.channel` (never in
  `sync::policy`). **Stable** = tags with no prerelease part; **Release Candidate** = those plus `rc`
  tags (so it moves on to `0.6.0`). Nothing saved: a build starts on its own channel.
- `update_check::check` reads `releases?per_page=30`, up to three pages (`/releases/latest` omits
  prereleases; the list is ordered by creation, so RCs can push the newest stable past page one).
  Stops after the first page holding an accepted release not newer than the running build; skips
  drafts and unaccepted/unparseable tags; takes the highest version. Strict SemVer: only dotted `rc.N`
  is an RC (`rc1`, `RC.1` offered nowhere). Never offers anything not newer than the running build, so
  a channel switch cannot downgrade. Every trust rule below applies to RCs.
- New channel: a `Channel` variant, its `accepts` rule, a label. `Kind` knows `rc`; anything else is
  "other", offered nowhere.
- Build id (`build.rs`): `dev` for debug builds (so a commit/branch switch does not recompile the
  library before tests), else the short commit hash, `-dirty` with uncommitted changes, `unknown`
  without git; `DEGENVIEW_BUILD_ID` overrides. Shown in Settings ▸ About and the copied build info.
  Informational: nothing reads it back, never sent anywhere. Each release has its tag commit's id.

1. Land on `main`; require a clean tree plus test, clippy, and fmt checks.
2. Preview with `tools/release.sh <version> --dry-run`.
3. Run `tools/release.sh <version>` (`--yes` skips confirmation): creates `release/v<version>`,
   bumps Cargo files, opens a PR, merge-commits it, then creates and pushes a signed tag on merged
   `main`. Hyphenated versions publish as GitHub prereleases; only Release Candidate offers `-rc.N`.
4. The tag-triggered Release workflow verifies tag identity/version, builds macOS arm64/x64 (zip +
   disk image each), Windows x64/ARM64 (zip + per-user installer each), Linux x64/arm64 tarballs and a
   Flatpak per Linux CPU, then publishes. Watch with `gh run watch`; confirm all twelve assets.

Release notes: generated from `git log --no-merges` (no changelog file). Stable lists everything
since the previous stable tag (`0.6.0` repeats `0.6.0-rc.1`); an RC only what is new since the
previous tag. The workflow runs from the tag's commit, so a notes-step change must be on `main` first.

Release builds restore **no build cache**: whatever could write one would end up signed in the next
release. (`release-cache.yml` and its weekly glibc check are gone; `release.yml` runs that check on
every Linux build.) Slower is intended. `ci.yml` and check workflows use `rust-cache` on PRs only.

Flatpak: reusable `flatpak.yml`, called by `release.yml` as the `flatpak` job, which the `release`
job needs (bundle exists before the immutable release publishes). Builds from source in the Flatpak
SDK for x86_64 and aarch64, no shared cache. Also runs on PRs touching `packaging/flatpak/**`, weekly
on `main`, and on demand (`flatpak-check.yml`, x86_64 only; releases build arm64 too): the only
pre-tag manifest check. Dependency changes do not trigger it. Details and what is unverified:
[Flatpak](../flatpak.md). The updater ignores the Flatpak asset; it picks only macOS/Windows zip
suffixes.

Stapling fetches the ticket from Apple's CloudKit, which drops now and then (worst on `macos-x64`,
the Intel runner: "network connection was lost", `stapler` exit 68). `stapler staple` and
`stapler validate` retry separately, ~ten minutes total; if only `validate` cannot reach Apple, the
offline `codesign --verify` and `spctl --assess` checks still run. Still failing on a tag: re-run the
failed job; the tag stays. Retry is `retry` in `tools/retry.sh`, sourced by the app and disk-image
steps (steps share no shell state; a shared file prevents drift); `tools/test-retry.sh` runs in
`macos-dmg-check.yml`.

Never squash/rebase the release PR or reuse/move a published tag. Fix a bad release with a new
version. Re-run a failed tagged build through the Release workflow dispatch.

## Trust and packaging

- Installable updates need an annotated signed tag GitHub reports `verified: valid`, tagger email in
  `tag_signature::MAINTAINER_EMAILS`. Rotate keys on the maintainer account; to change email, first
  release a version trusting both. Failed verification (incl. no GitHub access) offers **View
  Release**; the workflow rejects the same failure before building.
- Only GitHub binds Windows/Linux asset bytes to the signed tag (macOS adds Developer ID). A token that
  can create releases could publish its own assets for a signed tag with no release yet (during the
  build, or after a failed run). Follow-up: signed manifest or Authenticode; until then never leave a
  signed tag without its release. Only the workflow bot is an accepted author (`RELEASE_AUTHORS`), so
  the maintainer's token cannot do it via the API either; but a token that can add a signing key can
  forge the tag: keep `admin:ssh_signing_key` off every token on a developer machine.
- Release must be immutable and authored by `github-actions[bot]`; a hand-made one (maintainer
  included) is manual-download only, as are old mutable releases. Keep Immutable releases enabled.
  Upload assets before publication, never afterward.
- Keep suffixes `-macos-arm64.zip`/`-macos-x64.zip` with the `DegenView.app` layout, and
  `-windows-x64.zip` with `degenview.exe` at the zip root. The updater takes only the asset named
  exactly `DegenView-<tag><suffix>` and checks GitHub's digest. The release job requires `dist/` to
  hold exactly the twelve expected names; `gh release create "$TAG" dist/*` drafts, uploads and
  publishes; the workflow verifies release and assets last.
- macOS signing: Developer ID Application certificate in a throwaway keychain, sign via
  `tools/bundle.sh`, notarize with `notarytool` (App Store Connect API key), staple the `.app`, then
  zip. Secrets: README. Each bundle embeds its own Developer ID provisioning profile
  (`embedded.provisionprofile`); without it macOS kills the app at launch (`keychain-access-groups` is
  restricted). Profiles expire in 2044 or with the certificate. Access group
  `<TEAMID>.es.nico.degenview.shared`; `keychain_webhook_secrets::ACCESS_GROUP_SUFFIX` must match.
- After unpacking: `codesign --verify --deep --strict` with a Developer ID Application requirement
  pinned to the running app's Team ID (app and login item), then `spctl --assess` (`code_signature`).
  No Team ID (unsigned/ad-hoc) = not installable, **View Release**. Verified in the staging folder next
  to the app, not re-run at swap time.
- Downloads: only HTTPS redirects on GitHub/githubusercontent hosts; bounded archive entries and size;
  unsafe paths rejected; `ditto` limited; both app/login-item bundle ids, the executable set and links
  validated. Update `update_archive` constants when bundle layout changes.
- The helper clears its environment. `self_update::FORWARDED_ENV` holds only non-secrets: `HOME`,
  `DEGENVIEW_CACHE_DIR`, `DEGENVIEW_NO_KEYCHAIN`, `DEGENVIEW_NO_NOTIFICATIONS`. Never forward
  credentials; helper argv is visible to other users.

## Build hardening

Keeps a hostile change out of a signed release; each rule's reason is in `release.yml`.

- **Every `uses:` pinned to a commit hash**, version in a trailing comment
  (`actions/checkout@<40 hex> # v7`). Dependabot (`.github/dependabot.yml`) proposes hashes; `zizmor`
  (`supply-chain.yml`) fails a PR adding an unpinned one. No third-party action in a job with secrets
  or on the release path without a reason.
- **Permissions per job.** Default `contents: read`; only `release` gets `contents: write` (plus
  `id-token`, `attestations` for provenance). Checkouts use `persist-credentials: false`.
- **One commit.** `verify` resolves the signed tag to a commit in `main`'s history; later jobs check
  out that hash, not the tag name; `release` checks the tag still names it.
- **Signing material last.** macOS compiles the dependency graph (`Build macOS release binaries`) and
  installs `dmgbuild` before importing the certificate, so no build script or proc-macro sees the
  Developer ID or notarization key. `tools/bundle.sh` skips its build when `DEGENVIEW_SKIP_BUILD` is
  set. New macOS steps running third-party code go before the import.
- **`--locked` everywhere** a release or the Flatpak is built.
- **No build cache** in release jobs; compiler is `RUST_TOOLCHAIN` (exact version, bumped by its own
  PR), not the day's `stable`.
- **pip hash-locked**: `tools/requirements/dmgbuild.txt`, `flatpak-generator.txt`, with
  `--require-hashes`. Regenerate: resolve with `pip download`, take each file's sha256 from PyPI,
  review the diff. Flatpak container image pinned by digest in `flatpak.yml` (update tag and digest
  together); Flatpak `prepare` runs on `ubuntu-24.04`, not `-latest`, matching the generator file's
  Python.
- **Provenance.** `release` attests every asset (`actions/attest-build-provenance`); verify with
  `gh attestation verify <file> --repo degenview/degenview`. Not a substitute for an app-checked
  signature (follow-up in "Trust and packaging").
- **Dependencies.** `supply-chain.yml` runs `cargo deny` (`deny.toml`: RustSec advisories, yanked
  crates, crates.io only, license allow list, banned TLS stacks) on every PR and weekly. An `ignore`
  needs a reason. Locally: `cargo install cargo-deny --locked && cargo deny check`.
- **Repository settings** (not settable by workflows): a `release` environment with the signing
  secrets, deploying only from `v*` tags; a ruleset blocking creating/moving `v*` tags except by the
  maintainer; required status checks on `main`; "Require actions to be pinned to a full-length commit
  SHA". Public side: `SECURITY.md`.

## macOS disk image

`DegenView-<tag>-macos-<arch>.dmg` is the install download (`DegenView.app`, Applications shortcut,
arrow). The zip holds the same app and is what the updater downloads; the updater never reads the
image, and the `.dmg` name can never match `-macos-<arch>.zip` (test-pinned). Keep both.

- **Build**: `tools/make-dmg.sh <app> <out.dmg>`, dmgbuild pinned with all dependency hashes in
  `tools/requirements/dmgbuild.txt` (`--require-hashes` in `release.yml` and `macos-dmg-check.yml`;
  1.6.5 because later needs Python 3.10, Xcode's is 3.9); settings `packaging/macos/dmg-settings.py`.
  dmgbuild writes the Finder layout headless; `create-dmg` scripts Finder and is flaky on CI.
  `make-dmg.sh` signs nothing, so works on unsigned `target/DegenView.app` from `tools/bundle.sh`.
- **Release**: built from the already signed, notarized, **stapled** app; the image is then signed
  (Developer ID), notarized, stapled, and `spctl`-checked along with the app inside after attaching.
  Reuses the App Store Connect key from the app's notarize step, removed with other signing material.
- **Art**: `tools/make-dmg-background.py` (Pillow) writes `packaging/macos/dmg-background.png` and
  `@2x`, both committed, regenerated only when art changes. `make-dmg.sh` joins them into a
  Retina-aware TIFF (`tiffutil -cathidpicheck`), hence 72/144 dpi. Settings-file icon positions and
  the art's arrow change together.
- **Layout and Finder's own chrome**: Finder ignores hiding status/path/tab bars (user's global
  settings; sidebar/toolbar flags honoured), so with all on only ~256 of 380 points show: icons,
  labels and hint sit in the top 256. Looked at by hand on macOS 27 with bars on: all visible, 2x
  sharp.
- **Mid-tone background** on purpose: label colour follows the user's appearance. Light appearance
  checked only.
- **Running from the image**: read-only volume, updater says "Drag DegenView from the disk image to
  the Applications folder first." (`not_writable_message`).
- **CI**: `macos-dmg-check.yml` builds around a stub app on PRs touching it; checks app, Applications
  link, background (both sizes), layout file, volume icon, and that an ad-hoc seal verifies inside the
  image and after a `ditto` out. Cannot judge looks. By hand, the same check passed on a real Developer
  ID bundle with login item and profile (identical file modes after copy).
- **Verified on CI**: `macos-dmg-check` passes with dmgbuild 1.6.5 on `macos-latest` (arm64) Python
  (PR #247).
- **Not verified**: image signing/notarization/stapling, Gatekeeper on a quarantined download, the
  Intel leg (`macos-15-intel`), until an RC goes through the workflow. Dark appearance not looked at.

## Homebrew

`brew install --cask degenview/degenview/degenview` (final) and `.../degenview@rc` (final or `-rc.N`,
the Release Candidate set), from tap `degenview/homebrew-degenview` (checkout
`~/Developer/homebrew-degenview`). Homebrew 7 trusts a fully named cask as it installs; a short name
after `brew tap` needs `brew trust`. Like the Flatpak repo the tap **pulls** (six-hourly workflow);
nothing here holds a token for it. Never add a "bump the tap" step to `release.yml`.

- Casks install the `-macos-<arm64|x64>.zip` (not `.dmg`): zip name and root `DegenView.app` are a
  tap contract too.
- Tap's `scripts/bump.sh` verifies each zip (`gh release verify`, `verify-asset`,
  `gh attestation verify --signer-workflow .../release.yml`) and writes the downloaded bytes' sha256.
  Ignores releases not by the workflow bot and tags other than final or `-rc.N`.
- `auto_updates true`: the in-app updater replaces the app; `brew upgrade` skips it. `/Applications`
  is user-writable for an admin, so `install_target` passes.
- `depends_on macos: :sonoma` mirrors `LSMinimumSystemVersion` (14.0) in `tools/bundle.sh`; change
  together. `zap` lists `es.nico.degenview` under Application Support, Caches, Preferences, Saved
  Application State (`APP_DIR` in `store/mod.rs`); update if those move.
- **Verified**: `scripts/bump.sh` against v0.6.0-rc.15 (all steps pass, hashes equal API digests);
  `brew style`, `brew audit --cask --strict --online` (only "is a GitHub pre-release" for rc) and
  `brew fetch --cask` (hash checked) on a local tap copy, Homebrew 7.0.12, macOS.
- **Not verified**: `brew install`, `brew uninstall --zap`, in-app update of a brew install, the
  Intel cask, the tap's GitHub workflows (never run on GitHub), the stable cask against a real final
  release (none exists; text audited with the rc file's contents).

## Linux glibc baseline

Linux binaries must start on **glibc 2.35** (Ubuntu 22.04, Debian 12, Fedora 36+), so Linux release
legs run on `ubuntu-22.04` and `ubuntu-22.04-arm`, not `ubuntu-latest`: a newer build host silently
raises the floor. Older distros (RHEL 9 and clones, glibc 2.34) use the Flatpak.

- **What went wrong**: rc.11 tarballs built on Ubuntu 24.04. Per `.gnu.version_r`: **arm64** requires
  `GLIBC_2.39` outright (refused on 22.04); **x64** only *weakly* references it (std treats
  `pidfd_spawnp` as optional; `VER_FLG_WEAK`, tolerated), hard floor already `GLIBC_2.35` (`libm`).
  Read from the files, not run on 22.04.
- **The gate**: `tools/check-glibc.py <binary> [max]` (default 2.35) fails when a *required* version
  exceeds the baseline; weak-only are reported. Not `strings | grep GLIBC_` (cannot tell weak from
  required). `release.yml` runs it on each Linux build before publishing; the 22.04 link already
  refuses newer symbols, so it mainly guards against host drift. `linux-glibc-check.yml` builds on both 22.04 runners for PRs changing
  `Cargo.toml`/`Cargo.lock`, the check, or those workflows: catches a dependency that no longer
  builds/links on 22.04 (`Test linux` uses 24.04), proves the check can fail, and runs
  `tools/test-check-glibc.py` (small ELF files: weak vs required; damaged file gives an error, not a
  traceback).
- **No cache on release builds**, so no build script or C object (`ring`, bundled SQLite) from a
  newer image leaks its glibc requirement.
- **Changing the baseline** is a decision: edit `tools/check-glibc.py`, release runners,
  `linux-glibc-check.yml`, README. When 22.04 runners retire, build in an `ubuntu:22.04` container (or
  `cargo-zigbuild` with a glibc suffix), never move the baseline by accident.
- **Not verified**: first build on 22.04 (this PR's `linux-glibc-check` run), whether arm64 then
  passes, a real start on an old distro.

## Windows and Linux updates

Windows x64/ARM64 (`DegenView-<tag>-windows-x64.zip`, `-windows-arm64.zip`) and Linux x64/arm64
(`DegenView-<tag>-linux-<arch>.tar.gz`) share `service/self_update/portable_binary.rs`: replace the
one executable instead of macOS's `.app` swap. Per OS only unpacking
(`update_archive::{extract_exe, check_exe}` zip, `{extract_tar_gz_binary, check_elf}` tarball),
relaunch and wait differ; check, tag verification, download, digest and `UpdateHost` are shared. A
build takes its own CPU's asset; `check_exe` matches PE machine to the running CPU (native ARM64
refuses x64 and vice versa). An **x64 build emulated on Windows on ARM** stays x64 until the user
installs the ARM64 installer over it (same `AppId`) or unzips the ARM64 zip. Asset names and root
`degenview.exe` / `./degenview` are updater contracts.

### Windows

- **Trust**: signed tag, immutable release, GitHub SHA-256 (macOS's root). **No** Authenticode: the
  build is unsigned (no certificate, no CI step). Follow-up: a signer pin like macOS's Team ID.
- **Unpack**: `zip` crate in-process (inflate only), no `ditto`. `precheck` bounds the directory;
  `extract_exe` takes only root `degenview.exe`, fixed output name, `MAX_UNPACKED_BYTES` cap; all else
  ignored. `check_exe` requires a PE image for the running CPU.
- **Installable**: exe folder writable (`Program Files` is not: **View Release**); a source-tree build
  (`cargo run`) never replaces itself (paths compared without the `\\?\` prefix `canonicalize` adds).
- **Swap**: rename the running exe into `.DegenView-update\old\`, rename the new one in, roll back if
  that fails. If roll-back fails, the error says where the old copy is and how to restore it
  (`self_update::swap_paths`, shared); the next `prepare` won't clear staging while the app is missing
  and `old/` exists (`keep_only_copy`): it is the only copy. Same-named zip entries are refused
  (`has_duplicate_names`); an unreadable tree or extraction is an error, not a small size. Next start
  deletes staging (files in use wait one more start).
- **Relaunch**: no helper. New exe starts detached with `--relaunch-after <pid>` and a cleared
  environment (`FORWARDED_ENV` in `portable_binary`: data/cache folders, standard Windows child
  variables, non-secret `DEGENVIEW_*` switches, case-insensitive; never credentials).
  `wait_for_predecessor` runs first in `run_gui_with`, before the database opens, up to 60 s (as the
  macOS helper) so two instances never share the store. **Pid still alive at the end: fails**
  (`WAIT_TIMEOUT`); `run_gui_with` returns "The previous DegenView is still running. Close it, then
  open DegenView again." (`report_fatal`), database never opened.
- Inferred, not observed: our download has no Mark-of-the-Web, so SmartScreen should only hit the
  first manual download.
- **Not verified**: written/unit-tested on macOS only; `cfg(windows)` code (spawn, wait) and the
  real-process rename test run on CI `test-windows`. No real Windows update yet: release the next
  candidate, install the previous candidate's zip in a folder you own, update. Pre-updater builds must
  be replaced by hand once.
- **Windows ARM64**: native on `windows-11-arm` (free for public repos), matrix leg `windows-arm64` in
  `release.yml`; `ci.yml`'s `test-windows` runs there too, so `ring` (rustls), bundled SQLite (`cc`),
  `winresource` and the windowing stack build for `aarch64-pc-windows-msvc` on every PR. If `ring`
  needs a missing tool, fix the workflow (install LLVM, set `CC`), not the code.
- **Not done**: Authenticode signing, winget/Scoop manifests (Homebrew: see above).

### Linux

Hand-installed tarball (README: `~/.local/bin/degenview`). Trust as Windows; binaries unsigned.

- **Unpack**: `extract_tar_gz_binary` (`tar` crate, no default features, over `flate2`). Decompressed
  stream capped at `MAX_UNPACKED_BYTES`, at most `MAX_ENTRIES`; any unsafe name (absolute, `..`,
  non-UTF-8) refuses the archive; only regular file `degenview` (leading `./` stripped) is written,
  `0755`, fixed name. A symlink/hardlink of that name, or two, refuses it. Icon, `degenview.desktop`,
  `licenses/` are **not** updated. `check_elf`: 64-bit little-endian ELF executable or PIE for the
  running CPU.
- **Installable**: canonicalised binary's folder writable (`/usr/bin` distro install or read-only
  mount: **View Release**, hint `~/.local/bin`); never a development build; never a Flatpak
  (`sandbox::is_flatpak()` checked first, so a bundle Flatpak keeps its "download the new .flatpak"
  message although it is Linux x86-64 and a tarball exists).
- **Swap**: rename into `.DegenView-update/old/`, rename new in, with rollback (old process keeps its
  open inode).
- **Relaunch**: own process group, `--relaunch-after <pid>`, **whole environment inherited** (unlike
  Windows: cleared/allowlisted; macOS: `open --env` puts values on argv, hence its allowlist). A child's
  environment is invisible to other users and must carry `DISPLAY`/`WAYLAND_DISPLAY`/
  `XDG_RUNTIME_DIR`/`DBUS_SESSION_BUS_ADDRESS` (Secret Service), so an allowlist is riskier. Nothing on
  the command line. A script-based test observes both behaviours.
- **Wait**: `wait_for_predecessor` polls `/proc/<pid>/stat` every 100 ms up to 60 s; zombie (`Z`) =
  exited. No `libc`; no `/proc` = no wait.
- **Flatpak**: cannot self-replace or know its source (no remote in `/.flatpak-info`), so the note is
  `sandbox::FLATPAK_UPDATE_NOTE`: `flatpak update` (repo `degenview/flatpak` at
  `flatpak.degenview.app`) or download the bundle. Each release builds one branch from the tag
  (`stable` final, `beta` `-rc.N`, `dev` alpha/beta, unpublished); the repo also copies finals onto
  `beta`. See [Flatpak](../flatpak.md#hosted-repo).
- **Checked against real artifacts**: rc.11 tarballs (both CPUs) unpack via `extract_tar_gz_binary`
  with ELF machine `0x3E`, `0xB7`: ignored test `a_real_linux_release_tarball_unpacks` with
  `DEGENVIEW_TEST_LINUX_TARBALL=<path>`.
- **Not verified**: written on macOS; unit tests (extraction, ELF check, environment, `/proc` wait,
  swapping a held-open binary) run on Linux CI (x86-64), nothing on arm64. No real update yet: release
  the next candidate, install the previous candidate's binary into `~/.local/bin`, run under X11 and
  Wayland, update, check relaunch (display, Secret Service) and new build id in Settings > About.
  Pre-updater builds must be replaced by hand once.

### Panics on Windows

GUI-subsystem exe, no console: a panic would vanish silently. `src/crash_report.rs` installs a panic
hook first thing in `main`:

- `crash.log` in the data directory (`%APPDATA%\es.nico.degenview`), appended per panic (message,
  location, thread, backtrace); capped at 256 KiB (an entry that won't fit restarts it), entries cut
  to 64 KiB.
- Main-thread panics also show a message box ("DegenView stopped unexpectedly") naming the file only
  if written (full/unwritable directory: no path). Worker-thread panics are logged, not announced (MCP
  server and Pine runners catch some on purpose).
- Release builds are stripped: backtraces are mostly addresses; message and location carry it.

Not verified on Windows: the `MessageBoxW` path is `main`'s existing startup-error one, new title.

### Windows installer

`DegenView-<tag>-windows-x64-setup.exe` / `-windows-arm64-setup.exe` via
`tools/windows-installer.sh <tag> <exe> [x64|arm64]` from `packaging/windows/degenview.iss` (`Arch`
define), Inno Setup pinned in `release.yml` and `windows-installer-check.yml` (change both).
Unsigned. The zip (`-windows-x64.zip` / `-windows-arm64.zip`) stays as the updater asset; `-setup.exe`
names never match it (test-pinned, both CPUs).

- **Two installers, one product.** x64 is `x64os` (**not** Windows on ARM; `x64compatible` would
  install the emulated build there); ARM64 is `arm64`. Same `AppId`: ARM64 over x64 replaces in place,
  one Installed apps entry. `windows-installer-check.yml` runs on both runners (ARM installs ARM64 for
  real) and checks the x64 installer installs nothing and fails on ARM.
- **Per-user only, on purpose.** `PrivilegesRequired=lowest` → `%LOCALAPPDATA%\Programs\DegenView`,
  app-writable as the self-updater needs; `Program Files` could not self-update. Do not add
  `PrivilegesRequiredOverridesAllowed`.
- **`AppId` is permanent**: installer identity and Installed apps entry
  (`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}_is1`); changing it installs beside the
  old one. `INSTALLER_APP_ID` in `installed_version.rs` must equal it (a test reads the `.iss`).
- **Installed apps version.** Self-update swaps the exe, not the entry, so `sync_installed_version`
  (startup, beside `cleanup_stale`) rewrites `DisplayVersion`, only when Inno's `App Path` is this
  exe's folder (portable copies never touch an install's entry). Missing key = portable, ignored.
- **Installs** `degenview.exe` (always, `ignoreversion`: no version resource), OFL font licences in
  `licenses\`, Start menu shortcut, optional desktop one (off), GPL licence page, launch-after-install
  box (not in silent installs). `CloseApplications` asks a running DegenView to close.
- **Uninstall** removes the app folder (incl. `.DegenView-update`) and shortcuts; **keeps** user data
  (`%APPDATA%\es.nico.degenview`) and cache.
- **CI.** `windows-installer-check.yml` builds around a stub exe on PRs touching it, silent install,
  checks files/shortcut/uninstall entry, uninstalls, checks user data survived. Registry sync has a
  Windows-only unit test on `test-windows`.
- **Not verified**: wizard pages, shortcuts in a real session, closing a running app on upgrade,
  SmartScreen on the unsigned installer, the full loop (install rc.N, self-update to rc.N+1, see
  rc.N+1 in Installed apps, uninstall). CI's silent install/uninstall is the only run.

## iCloud entitlements

`DEGENVIEW_ICLOUD=1` (release: repository variable `DEGENVIEW_ICLOUD`) builds `--features icloud` and
signs only the app with `tools/entitlements-app.plist`; the login item never gets iCloud entitlements.
The app profile must grant them or macOS kills the app at launch: ship the first iCloud build as a
hyphenated prerelease and run `tools/verify-icloud.sh`. The Swift bridge links into the `degenview`
binary; add no library or framework to the bundle (the updater accepts only the two known
executables). Deploy Production's CloudKit schema before a release that changes records. Details:
[iCloud Sync](../icloud-sync.md).

## In-app update state

- `UpdateHost` is the sole check/install/restart state, app-owned, exposing a cloned `UpdateView` to
  windows and Settings; nothing else calls check/prepare. Memory-only.
- Check ~15 seconds after launch, every six hours, or 30 minutes after a failed attempt (also while a
  known update is on offer). Not while installing or awaiting restart; never discard a known update
  after a failed check. A failed install keeps the schedule: its error belongs to that release, kept
  by scheduled checks while the same release is offered, dropped for a different one; only a person's
  check or a channel switch clears it directly.
- `install_target` (runs `codesign` on macOS) is looked up on the update worker by the first tick, not
  at app start; until then an available update says so and is not installable.
- Tab-bar indicator and About card render `UpdateView::Phase`; UI never reads host internals. The
  indicator reserves strip space. Progress: 0–5% signature, 5–90% download, 90–100% unpack. Workers
  repaint only viewports that drew the state.
- Prompt the focused window once when ready. “Later” leaves the Restart indicator; restart uses
  `AppAction::Quit` so unsaved-script checks run.
- **Order of a restart**: "Restart Now" only *swaps* (`self_update::apply`, once; failure shows on the
  tab bar) and arms the relaunch. The new version *starts* from `eframe::App::on_exit`
  (`UpdateHost::relaunch_if_armed`, `self_update::relaunch`) when exit is certain. Never earlier: that
  put a second process beside the old one when the unsaved-script question was slow or cancelled.
  `call_off_quit` disarms it (a later ordinary quit starts nothing); a second "Restart Now" after a
  cancel skips the swap (swapping again would move the new version over the kept copy). The macOS
  helper also starts the app only if the old process is gone when its 60 s wait ends. A spawn failure
  in `on_exit` is only logged; the user reopens the app. Not run end-to-end on any OS yet.
