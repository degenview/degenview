# Auto-update code review

Date: 2026-11-02  
Scope: update discovery, release trust, downloading, archive validation, installation, relaunch,
packaging, and CI for macOS, Windows x64/ARM64, Linux x64/ARM64, and Flatpak.  
Review type: static review plus focused unit tests; no implementation changes.

## Executive summary

The updater is unusually careful for an in-house implementation. It separates discovery from
installation, keeps long-running network and archive work off the UI thread, verifies GitHub's
asset digest, restricts download redirects, bounds archive expansion, validates target CPU, pins a
macOS update to the running app's Developer ID team, and has useful state-machine and archive tests.
The portable and app-bundle strategies are also cleanly separated.

I found no critical issue that lets an unauthorised network peer directly install an update. The
largest security concern is instead the Windows/Linux trust model: the signed git tag does not sign
the release assets, while the asset digest and immutability assertions come from the same GitHub
control plane that hosts the bytes. macOS has an independent Developer ID check; Windows and Linux
do not. This is documented in the repository, but it remains the most important residual risk.

The main correctness risks are recoverability around the two-rename swap, an install failure
disabling automatic checks for the rest of the process, and discovery using only the first 30
GitHub releases. There are also several hardening and maintainability opportunities. None requires
an emergency release, but the high and medium findings should be addressed before treating the
updater as fully mature on Windows and Linux.

Severity meanings used below:

- **High:** can cross an important trust boundary or leave an installation unusable under a
  realistic privileged-compromise/failure scenario.
- **Medium:** user-visible correctness, availability, or defence-in-depth issue with a plausible
  trigger.
- **Low:** hardening, edge-case correctness, maintainability, or test coverage improvement.

## Findings

### High — Windows and Linux assets are not cryptographically bound to the signed tag

Affected: Windows x64/ARM64 and Linux x64/ARM64 portable updates.

`tag_signature::verify_release_tag` proves that the selected tag object is signed and targets a
commit (`src/service/tag_signature.rs:88-143`). The downloaded archive is then compared with the
SHA-256 value GitHub reports for the release asset (`src/service/self_update/mod.rs:119-128`). That
detects corruption and substitution in transit, but the digest is not part of the signed tag. The
release author, immutability flag, asset URL, asset digest, and asset bytes all come from GitHub.

Consequently, a compromise that can publish release assets under the workflow bot or maintainer
account can attach an arbitrary Windows/Linux binary to an existing valid signed tag before making
the release immutable. The tag check still succeeds and GitHub truthfully reports the attacker's
asset digest. macOS has a second trust boundary because `code_signature::verify_bundle` requires a
Developer ID signature from the running app's Team ID (`src/service/code_signature.rs:30-50`);
Windows and Linux deliberately perform only PE/ELF structural checks
(`src/service/update_archive.rs:219-226`, `326-334`).

Recommendation: publish a manifest containing the exact tag, asset names, sizes, and SHA-256
digests and sign that manifest with an offline/release key whose public key is embedded in the app.
Alternatively, add Authenticode verification with a pinned publisher on Windows and an equivalent
detached-signature scheme for Linux. GitHub attestations can be an additional signal, but only if
the client verifies a trust root independent from the release metadata it is downloading.

This is already acknowledged in `docs/agent-guides/releases-updates.md`; the finding records the
residual security consequence rather than a documentation gap.

### Medium — A failed or interrupted swap can leave the application missing

Affected: macOS, Windows, and Linux self-installing builds.

Both install strategies move the running installation to `.DegenView-update/old/` and then move the
new installation into its original path (`src/service/self_update/app_bundle.rs:165-175`,
`src/service/self_update/portable_binary.rs:188-202`). If the second rename fails, rollback is
attempted, but the rollback result is discarded. The reported error mentions only the failed new
rename even if restoring the old installation also failed. A crash, power loss, filesystem fault,
endpoint-security intervention, or concurrent filesystem mutation between the two renames has the
same outcome: the normal launch path may no longer exist.

The old installation is normally recoverable from the staging directory, so this is application
availability rather than user-data loss. The UI does not expose the recovery path, however, and a
failed rollback is indistinguishable from an ordinary installation failure.

Recommendation: preserve and report both errors; add startup/helper recovery that restores `old`
when the canonical app path is absent; fsync relevant files/directories where the platform supports
it; and test injected failure/crash points between every filesystem transition. On macOS, evaluate
`renameatx_np(..., RENAME_SWAP)` for an atomic exchange. Windows will still need a helper/recovery
protocol because an atomic exchange of the running executable is not generally available.

### Medium — One install failure stops scheduled checks for the rest of the session

Affected: all platforms.

`check_due` refuses to check unless `Install::Idle` (`src/host/update_host.rs:499-513`). A failed
prepare changes the state to `Install::Failed` (`src/host/update_host.rs:349-361`), and nothing
returns it to `Idle` automatically. The user can manually retry or manually check again, and a
restart reconstructs the host, but the documented periodic checker will otherwise never discover a
newer corrected release during that process lifetime. The existing test explicitly locks in this
behaviour at `src/host/update_host.rs:782-789`.

Recommendation: block checks only while work is active or a prepared update awaits restart. Keep
the install error visible independently from discovery state, and allow the normal retry schedule
to replace a failed release with a newer one. Add a state-machine test covering “install N fails,
N+1 is published, periodic check discovers N+1.”

### Medium — Release discovery can miss valid updates after 30 releases

Affected: all platforms, especially the Stable channel.

Discovery requests exactly one page with `per_page=30` and does not follow pagination
(`src/service/update_check.rs:10-12`, `164-195`). Filtering by channel happens only after that page
is downloaded. If more than 30 release candidates, alpha/beta builds, or otherwise ineligible
releases were published after the newest stable release, a Stable user on an older build sees
`UpToDate` even though an applicable stable release exists on the next page. A similar problem can
affect any channel after enough unsupported releases.

Recommendation: follow GitHub pagination until an accepted version is found that is newer than the
running build, or until there are no more pages. Bound the number of pages and retain the current
body/timeout limits. Add a test with more than 30 ineligible releases ahead of a valid stable one.

### Medium — macOS update eligibility can block application startup

Affected: macOS.

`DegenViewApp::new` constructs `UpdateHost`, which immediately calls `install_target`
(`src/host/update_host.rs:116-135`, `src/app.rs:744`). On macOS that performs a write probe and runs
`codesign -dv` through a polling helper whose timeout is 120 seconds
(`src/service/self_update/app_bundle.rs:34-64`, `src/service/code_signature.rs:14-27`, `89-123`).
This happens synchronously in the application creation callback, before the regular updater worker
is involved. It is normally quick, but a stalled process, overloaded disk, or security tooling can
delay the whole launch for up to two minutes.

Recommendation: initialise the updater as “eligibility unknown,” determine the install target and
Team ID on the updater worker, and publish the result into `UpdateHost`. Eligibility should not be a
prerequisite for showing the first window or checking whether an update exists.

### Medium — Release verification happens only after the immutable release is public

Affected: release engineering and all clients.

The release workflow publishes all assets with `gh release create` and only afterwards runs
`gh release verify` and `gh release verify-asset` (`.github/workflows/release.yml:410-455`). If that
last step fails permanently, the release has already been visible to clients and is intended to be
immutable. The updater does not consume the attestation checked by this step; it checks the release
metadata, tag, digest, and platform structure. Therefore the verification job is a useful alarm but
not a publication gate, and there is an exposure window even when the workflow eventually fails.

Recommendation: validate the complete downloaded artifact set before publication using locally
computed checksums, archive inspection, platform signatures, expected versions, and expected asset
inventory. Treat GitHub's post-publication attestation verification as an additional audit. If the
platform cannot attest an immutable release before publishing it, document the recovery procedure
and ensure a failed verification immediately creates a prominent security incident rather than
only a failed workflow.

### Medium — The client does not verify that an asset contains the advertised version

Affected: macOS, Windows, and Linux.

Asset selection requires only a suffix and chooses the first match
(`src/service/update_check.rs:127-151`); it does not require the exact
`DegenView-<tag>-<platform>` name or uniqueness. After extraction, macOS checks bundle IDs and
executables, Windows checks the PE machine, and Linux checks the ELF class/type/machine, but none
compares an embedded application version with `Release::version`. The release workflow's
Cargo/tag check and fresh build make an accidental mismatch unlikely, but a stale or misnamed
artifact could be accepted and repeatedly offered after installation.

Recommendation: require the exact expected asset name and exactly one match. Embed machine-readable
version/build metadata in every binary and compare it with the selected release before marking an
update ready. On macOS, parse `Info.plist` and validate `CFBundleIdentifier`,
`CFBundleExecutable`, and the compatible version rather than searching for ID strings
(`src/service/update_archive.rs:341-359`).

### Low — A failed refresh of a known update waits six hours, not 30 minutes

Affected: all platforms.

When a check fails while an earlier update is known, `finish_check` keeps the earlier
`UpdateOutcome::Available` so the UI does not lose it (`src/host/update_host.rs:298-313`). This is
good UI behaviour, but `check_due` chooses its next interval from the retained outcome, not from the
latest attempt. It therefore uses the six-hour success interval instead of the documented
30-minute failure retry.

Recommendation: track the last attempt result/time separately from the last useful release result.
Test that an available update remains visible while a failed refresh becomes due after 30 minutes.

### Low — Archive validators accept avoidable ambiguity and suppress inspection errors

Affected: macOS and Windows.

The Windows extractor asks `ZipArchive::by_name("degenview.exe")` and does not reject duplicate
entries with that root name (`src/service/update_archive.rs:158-183`). Linux explicitly rejects a
duplicate `degenview`, so the two portable formats have inconsistent ambiguity handling. The macOS
bundle walk and size walk use iterator flattening (`src/service/update_archive.rs:136-149`,
`363-388`), which silently drops `read_dir` entry errors. A validation walk should fail closed when
it cannot inspect the complete tree.

Recommendation: reject duplicate target entries during the zip directory precheck, propagate every
directory-entry and metadata error, and add fixtures for duplicate names and injected traversal
errors. Also validate the central-directory boundaries and expected compression methods explicitly
rather than relying on the downstream extractor for the final rejection.

### Low — Linux silently skips predecessor waiting when `/proc` is unavailable

Affected: Linux portable updates.

The relaunched process polls `/proc/<pid>/stat`; any read failure means the predecessor is treated
as gone (`src/service/self_update/portable_binary.rs:357-405`). On a normal desktop `/proc` is
present, but restricted containers, unusual mounts, or sandbox policies weaken the stated guarantee
that the old and new process never open the database together. PID reuse can also make a new
process wait for an unrelated process.

Recommendation: use a process handle/pidfd where available, pass an inherited lock/pipe from the
predecessor, or use the database/single-instance lock as the final authority. An inability to prove
the predecessor exited should fail closed with a useful error rather than proceed.

### Low — Stale cleanup and preparation can race on the same staging directory

Affected: all self-installing platforms.

Startup submits `cleanup_stale` to the four-worker general queue (`src/lib.rs:35-39`), while checks
and preparation use the independent single-worker update queue (`src/host/update_host.rs:290-345`,
`src/service/work_queue.rs:43-57`). Both may remove or create `.DegenView-update`. The initial
15-second delay makes the race unlikely, but opening Settings triggers an immediate check, and
there is no ordering guarantee between the queues.

Recommendation: run cleanup synchronously before update state is exposed, or enqueue cleanup on
the same update worker before any check/prepare job. Add a concurrency test that prevents cleanup
from deleting a newly prepared staging tree.

### Low — Version parsing is broader than the release contract

Affected: release discovery and channels on all platforms.

The parser is intentionally small but is not a strict SemVer 2.0 parser
(`src/service/update_check.rs:219-309`). It accepts forms the release guide forbids, including
`rc1`, repeated leading `v` characters, leading zero numeric identifiers, and some malformed build
or prerelease forms. `tools/release.sh` prevents the project from creating those tags, but the
client is parsing remote repository state and should share the same contract.

Recommendation: use a well-tested SemVer type or make the local parser strict, then classify only
the exact `rc.N` form accepted by release tooling. Add contract tests shared conceptually with
`tools/release.sh` for accepted and rejected tags.

### Low — The writable-directory probe has a predictable, destructive name

Affected: all self-installing platforms.

`is_writable` creates and then removes `.DegenView-write-test-<pid>`
(`src/service/self_update/mod.rs:112-117`). If an empty directory with that name already exists,
`create_dir` fails but `remove_dir` still deletes it. A stale collision also produces a false
“not writable” result.

Recommendation: use a securely unique temporary name with create-new semantics and remove only an
object proven to have been created by this invocation.

### Low — Trust and packaging contracts are duplicated manually

Affected: maintenance and release safety.

Maintainer identities, repository URLs, asset suffixes, installer IDs, architecture names, bundle
IDs, and platform matrices are repeated across Rust, shell, Inno Setup, workflows, and prose. Some
have tests, but important examples such as `MAINTAINER_EMAILS` versus the workflow's
`MAINTAINER_EMAIL`, release-author allowlists, and the complete asset inventory can drift without a
single failing contract test.

Recommendation: centralise stable metadata where practical and add one release-contract test that
parses the workflows/packaging definitions and asserts exact agreement with updater constants for
every supported OS/CPU. Keep security identities in a deliberately reviewed manifest rather than
unrelated string literals.

### Low — Downloads are buffered wholly in memory

Affected: all self-installing platforms.

`update_download::get` returns a `Vec<u8>` up to the advertised 256 MiB asset limit, after which the
macOS path also writes it to disk before extraction (`src/service/update_download.rs:16-34`,
`src/service/self_update/app_bundle.rs:119-138`). This is bounded and therefore not an unbounded
memory bug, but it can create a substantial transient memory peak on lower-memory machines.

Recommendation: stream to a create-new staging file while hashing and counting bytes, fsync it,
then extract from that file. Windows/Linux extractors can likewise read the verified staging file
instead of retaining the archive in memory.

### Low — Windows Installed Apps version synchronisation is best-effort and path-fragile

Affected: Windows installer builds.

Registry failures are silently ignored, and install matching compares lowercased strings with only
trailing separator normalisation (`src/service/self_update/installed_version.rs:22-49`, `173-185`).
Junctions, short/long path aliases, or unusual Inno paths can leave `DisplayVersion` stale even when
the running executable belongs to that installation.

Recommendation: compare canonical file identities or normalised Windows paths, log failed reads and
writes without making startup fail, and extend the Windows-only test to cover a real self-update
followed by registry synchronisation.

## Positive observations

- Release checks skip drafts and unsupported tag kinds, compare versions numerically, never
  downgrade, and preserve a known update across transient refresh failures.
- Installable releases require an allowed author, GitHub immutability, a bounded asset size, a
  GitHub release-download URL, and a syntactically valid SHA-256 digest.
- Downloads allow HTTPS only, revalidate every redirect, restrict hosts to GitHub and
  `*.githubusercontent.com`, disallow credentials/non-default ports, limit redirects and bytes, and
  enforce an overall timeout.
- macOS archive expansion is bounded by declared and observed output size and time. The bundle
  layout check rejects unexpected executable files and escaping links; Developer ID and Gatekeeper
  checks add a strong independent platform boundary.
- Windows extracts only the root executable and checks PE machine type; Linux extracts only the
  root binary, rejects duplicate/link forms, bounds the entire decompressed stream, fixes mode to
  `0755`, and checks ELF class, endianness, kind, and machine.
- Windows relaunch clears the environment and forwards a non-secret allowlist. macOS similarly
  limits both helper environment and command-line values. Linux's full environment inheritance is
  a reasoned desktop-session compatibility tradeoff and does not place secrets in argv.
- Applying an update is delayed until explicit restart, and relaunch waits until exit is certain;
  cancelling the unsaved-script quit disarms relaunch. The state machine has targeted tests for
  these transitions.
- Platform asset names, Windows installer suffixes, macOS DMG suffixes, ARM64 CPU selection,
  Flatpak exclusion, and Linux glibc baseline are covered by code or workflow tests.
- Flatpak's notify-only behaviour is intentional: downloaded bundles have no update remote. It is
  not a defect in the portable updater.

## Validation and coverage gaps

Executed:

```text
cargo test --locked --lib -- self_update update_archive update_check update_host

75 passed; 0 failed; 3 ignored; 5,159 filtered out
```

The ignored tests require GitHub or previously downloaded real release artifacts:

- latest macOS release download, signature check, unpack, and swap;
- real Windows release zip extraction;
- real Linux release tarball extraction.

The repository documentation also identifies the following end-to-end gaps, which remain important
because unit tests cannot reproduce OS launch services, endpoint security, locked files, or desktop
session environments:

- update and relaunch between two real signed/notarized macOS releases;
- installer -> self-update -> Installed Apps version -> uninstall on Windows x64 and ARM64;
- portable Windows update with a real running GUI process on both CPUs;
- Linux update and relaunch under both X11 and Wayland, including Secret Service access;
- Linux ARM64 runtime update;
- Flatpak window, networking, secret storage, and manual-update messaging;
- injected crash/power-loss and rollback-failure tests at each swap transition;
- update checks against paginated GitHub fixtures and release-publication failure drills.

## Suggested priority

1. Bind Windows/Linux assets to an independent signature and design recoverable swaps.
2. Fix the `Install::Failed` scheduling dead-end and paginate discovery.
3. Move macOS eligibility work off startup and validate artifacts fully before publication.
4. Tighten asset/version/archive contracts, then consolidate duplicated release metadata.
5. Run one release-candidate update loop on every supported OS/CPU before declaring the updater
   end-to-end verified.

## Validation

Every finding above was checked against the code (and, where it mattered, the zip 9.2.2 source and
GitHub's immutable-release documentation). The test run quoted above was not repeated verbatim; the
updater tests were run again after the fixes (see the pull request). Line numbers in the review have
drifted slightly: `select_asset` is `update_check.rs:155-171`, `check` is `:185-197`, and the
`Install::Failed` assertion in `update_host.rs` was at line 788.

| Finding | Verdict | Outcome |
| --- | --- | --- |
| High: Windows/Linux assets not bound to the tag | True, narrower than written: the exploit needs a signed tag with no release yet (immutability locks a published tag name, and a deleted release's name is retired). The `tag_signature.rs` claim that a write token "cannot ship an update" was wrong. | Docs corrected. Signed manifest or Authenticode is a separate design. |
| Medium: failed swap can leave the app missing | True, and worse on Windows/Linux: `prepare` on retry deletes `old/`, the only copy (macOS stops earlier by accident). "Startup recovery" cannot work: the missing binary is the one that would run it. | Not in this PR. |
| Medium: `Install::Failed` stops scheduled checks | True. `Failed` now carries its version; a scheduled check keeps the error for the same release and drops it for a different one. | Fixed. |
| Medium: only the first 30 releases | True. | Fixed: up to three pages. |
| Medium: macOS eligibility blocks startup | True. | Fixed: looked up on the update worker. |
| Medium: verification after publication | Overstated. An attestation exists only after publication and a draft cannot be verified; local checksums of bytes built in the same job prove nothing independent. | The real gap (no check of the asset set) is closed; the comment now says the step is an alarm, not a gate. |
| Medium: asset name/version not checked | Name: true. Embedded version: not done. | Exact name required. |
| Low: failed refresh waits six hours | True. | Fixed. |
| Low: archive validators | Duplicate zip entries and swallowed `read_dir` errors: true. Central-directory and compression checks: no concrete risk (the zip crate rejects both). | Not in this PR. |
| Low: Linux wait without `/proc` | True, already documented; no database lock exists to fall back on. | Not in this PR. |
| Low: cleanup/prepare race | Theoretical as written; a second app instance deleting the first one's prepared update is the real variant. | Not changed. |
| Low: version parsing | True, and the lenient `rc1` handling was deliberate but contradicted the release guide. | Fixed: strict SemVer, only `rc.N`. |
| Low: write probe | True, trivial. | Fixed. |
| Low: duplicated trust constants | Partly: no test tied `MAINTAINER_EMAIL` in the workflow to `MAINTAINER_EMAILS`. | Tests added; no central manifest. |
| Low: download held in memory | True; bounded at 256 MiB, real assets about 60 MB. | Not worth changing. |
| Low: Windows registry sync | True but cosmetic. | Not changed. |
