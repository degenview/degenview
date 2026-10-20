# Releases and updates guidance

## Release procedure

`Cargo.toml` is the only app-version source; About, update checks, and `tools/bundle.sh` read it.
Never edit the version or create a release tag by hand.

1. Land on `main`; require a clean tree plus test, clippy, and fmt checks.
2. Preview with `tools/release.sh <version> --dry-run`.
3. Run `tools/release.sh <version>` (`--yes` skips confirmation). It creates
   `release/v<version>`, bumps Cargo files, opens a PR, merge-commits it, then creates and pushes a
   signed tag on merged `main`. Hyphenated versions are prereleases and ignored by updates.
4. The tag-triggered Release workflow verifies tag identity/version, builds macOS arm64/x64,
   Windows, and Linux, then publishes. Watch with `gh run watch` and confirm all four assets.

Never squash/rebase the release PR or reuse/move a published tag. Fix a bad release with a new
version. Re-run a failed tagged build through the Release workflow dispatch.

## Trust and packaging

- Installable updates require an annotated signed tag that GitHub reports `verified: valid`, with
  a tagger email in `tag_signature::MAINTAINER_EMAILS`. Rotate keys on the maintainer account; to
  change email, first release a version trusting both. Failed verification, including no GitHub
  access, offers **View Release** instead; the workflow rejects the same failure before building.
- The release must be immutable and authored by `github-actions[bot]` or an allowed maintainer.
  Keep Immutable releases enabled. Assets must be uploaded before publication; never upload to an
  immutable release afterward. Old mutable releases are manual-download only.
- Keep macOS suffixes `-macos-arm64.zip`/`-macos-x64.zip` and the `DegenView.app` layout. The updater
  checks GitHub's digest. `gh release create "$TAG" dist/*` handles draft, upload, and publish; the
  workflow verifies the release and assets last.
- The Release workflow imports a Developer ID Application certificate into a throwaway keychain,
  signs through `tools/bundle.sh`, notarizes with `notarytool` (App Store Connect API key), staples
  the `.app`, and only then zips it. Secrets are listed in the README. Each bundle embeds its own
  Developer ID provisioning profile (`embedded.provisionprofile`); without one, macOS kills the
  signed app at launch because `keychain-access-groups` is restricted. Profiles expire in 2044 or
  with the certificate. The keychain access group
  is `<TEAMID>.es.nico.degenview.shared`; `keychain_webhook_secrets::ACCESS_GROUP_SUFFIX` must keep
  matching it. After unpacking, the updater runs `codesign --verify --deep --strict` with a Developer
  ID Application requirement pinned to the running app's own Team ID (app and login item), then
  `spctl --assess` for notarization (`code_signature`). A running app with no Team ID (unsigned or
  ad-hoc) is not installable and offers **View Release**. Verification happens in the staging folder
  next to the app, so it is not re-run at swap time.
- Downloads follow only HTTPS redirects on GitHub/githubusercontent hosts, bound archive entries
  and size, reject unsafe paths, limit `ditto`, and validate both app/login-item bundle ids, the
  executable set, and links. Update `update_archive` constants when bundle layout changes.
- The helper clears its environment. `self_update::FORWARDED_ENV` may contain only non-secrets:
  `HOME`, `DEGENVIEW_CACHE_DIR`, `DEGENVIEW_NO_KEYCHAIN`, and `DEGENVIEW_NO_NOTIFICATIONS`. Never
  forward credentials; helper argv is visible to other users.

## In-app update state

- `UpdateHost` is the sole check/install/restart state. The app owns it and exposes a cloned
  `UpdateView` to windows and Settings; no other code calls check/prepare directly. State is
  memory-only. Check about 15 seconds after launch, every six hours, or 30 minutes after failure.
  Do not check while installing/awaiting restart or discard a known update after a failed check.
- The tab-bar indicator and About card render `UpdateView::Phase`; UI never reads host internals.
  The indicator reserves strip space. Progress is 0–5% signature, 5–90% byte download, and
  90–100% unpack. Workers repaint only viewports that drew the state.
- Prompt the focused window once when ready. “Later” leaves the Restart indicator; restart uses
  `AppAction::Quit` so unsaved-script checks still run.
