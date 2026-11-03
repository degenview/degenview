# iCloud Sync code review

Date: 2026-11-02  
Scope: sync policy and projection, SQLite tracking and outbox durability, merge and apply logic,
account and device identity, the Rust host/state machine, the Swift `CKSyncEngine` bridge, privacy,
release documentation, and automated tests.  
Review type: static review plus the repository's automated checks; no implementation changes and no
new real-CloudKit or two-Mac run.

## Executive summary

The implementation has a strong foundation. SQLite remains authoritative, local writes and their
outbox intents are normally committed together, the core separates transport from merge policy,
records are structurally validated and size-bounded, payloads use CloudKit encrypted fields, and the
test suite covers an unusually broad set of convergence, conflict, crash, account, and first-sync
scenarios. The category policy also defaults unknown data to local-only on upload, and the native
bridge keeps CloudKit work off the app thread.

I nevertheless found three high-severity correctness and isolation defects. First, both host apply
paths acknowledge a fetched batch even when applying it to SQLite failed. That can advance the
CloudKit checkpoint past data that never reached the local database. Second, the outbound setting
allowlist is not enforced on inbound records, so a `setting.<key>` record already present in the
private zone can overwrite settings explicitly classified as local-only, including security and
consent state. Third, the copied-database device check runs only when the user presses **Enable**;
an already-enabled database copied or restored onto another Mac starts its worker with the original
Mac's device identity and checkpoint.

The main medium-severity gaps are cross-version deletion handling, deletion handling when a local
table is unreadable, the lack of transactional coupling between Pine script files and SQLite sync
metadata, ignored persistence errors at account/destructive boundaries, and non-atomic replacement
of local collections during **Use iCloud's data**. The Swift bridge's generic send-error path also
needs a real-engine fault test: the Rust core retains its in-flight markers, so forward progress
depends on `CKSyncEngine` producing per-record completion events or retrying the pending changes.

These issues do not invalidate the whole design, but the high findings should be fixed before iCloud
Sync is treated as production-safe. The current release documentation should also be reconciled:
`docs/icloud-sync.md` says real CloudKit has never been exercised while the release checklist and
performance section record successful Production and Development runs.

Severity meanings used below:

- **High:** can silently lose, misapply, or cross-isolate user data under a realistic failure,
  migration, or stale-cloud scenario.
- **Medium:** user-visible correctness, recoverability, availability, or future-compatibility issue
  with a plausible trigger.
- **Low:** bounded resource risk, documentation defect, hardening opportunity, or maintainability
  concern.

## Findings

### High — A failed remote apply is acknowledged and its checkpoint may advance

Affected: foreground application, covered/hidden-window worker application, every synced entity.

`Applier::apply` deliberately returns `None` when `apply_batch` fails and says the batch should not
be acknowledged (`src/host/sync_host.rs:224-240`). The two callers do the opposite:

- The app-thread path sends `WorkerMsg::Applied(id)` unconditionally after calling the applier
  (`src/host/sync_host.rs:400-409`).
- The hidden-window path calls `core.remote_applied(id)` unconditionally
  (`src/host/sync_host.rs:801-807`).

`SyncCore::remote_applied` removes the batch from `unapplied`, releases queued merges, and calls
`flush_state` (`src/sync/core.rs:861-870`). Once no batches remain, `flush_state` persists the pending
engine serialization as the new CloudKit checkpoint (`src/sync/core.rs:873-895`). A disk-full error,
SQLite constraint, damaged local table, or any later error in `apply_batch` can therefore roll the
transaction back and still tell the core it succeeded. The next launch starts after that change and
need not fetch it again.

The existing crash test gives false confidence at the integration boundary. It invokes
`apply_batch` directly, observes the error, and intentionally never calls `remote_applied`
(`src/sync/crash_tests.rs:104-169`). It proves the core invariant only when the caller honors the
contract; it does not exercise `SyncHost`, where the contract is broken.

Recommendation: make application success explicit. Send `Applied(id)` or call `remote_applied(id)`
only for `Ok(ApplyReport)`. On failure, retain or requeue the exact `RemoteBatch`, expose a persistent
error, and retry with bounded backoff; alternatively restart the fetch from the last durable
checkpoint. Add host-level tests for both the app-thread and idle-worker paths using an aborting
SQLite trigger. Assert that the checkpoint does not move, the batch is retried after the fault is
removed, and the remote row arrives exactly once.

### High — Inbound records can overwrite settings classified as local-only

Affected: device-local settings, including MCP, content consent, AI preferences, alert runtime
state, update channel, session state, and future unknown settings.

The outbound side is allowlisted correctly: `project_setting` returns an entity only when
`setting_policy` is `Portable` (`src/sync/project.rs:147-163`). The policy marks every unknown key
local and explicitly tests sensitive examples such as `mcp.settings`, `mcp.permissions`,
`ai.prefs`, and `update.channel` (`src/sync/policy.rs:246-254`, `310-342`). Existing tests only prove
that these keys are not queued or uploaded.

The inbound side does not enforce the same decision. Validation accepts every recognized
`EntityKind::Setting` whose payload is valid JSON and under 100 KB, without checking the key's
policy (`src/sync/validate.rs:61-74`). `apply_simple` then treats it as a normal setting, and
`compose_setting` returns the remote value for every key except `snapshot.prefs`
(`src/sync/apply.rs:411-475`, `1708-1729`). Thus a stale record left by a development build, a newer
or buggy client, manual CloudKit data, or a future policy change can write a key this version says
must never sync.

This is an isolation failure even though the database is private. For example, a cloud record named
`setting.content.prefs` can change whether this Mac fetches external post previews, and
`setting.mcp.permissions` or `setting.mcp.settings` can alter security-related local configuration.
Keychain secrets and MCP client verifiers remain in local tables, so this is not a direct credential
exfiltration, but the stated local-only boundary is not enforced.

Recommendation: reject or durably quarantine inbound `Setting` records unless
`setting_policy(id)` is `Portable`; apply the same check before deletion/merge logic. Centralize the
portable-key decision so projection and validation cannot drift. Add tests that inject remote
upserts and deletes for every representative local-only key plus an unknown future key, and assert
that neither the setting nor its sync metadata changes. Keep `snapshot.prefs`' per-Mac consent merge
test as a separate allowed case.

### High — An already-enabled database copied to another Mac keeps the old device identity

Affected: Migration Assistant, restored backups, manually copied databases, cloned development
profiles, and any move of an enabled database between Macs.

The intended protection exists in `account::check_device_identity`: when the stored hardware hash
differs, it mints a new device ID and drops the device-specific engine checkpoint
(`src/sync/account.rs:175-186`, `src/store/sync.rs:387-394`). However, `SyncHost::new` immediately
starts the worker whenever the stored state is already enabled (`src/host/sync_host.rs:268-303`). It
does not run the identity check first. The only production call is inside `SyncHost::enable`, after
the user presses the switch (`src/host/sync_host.rs:555-567`). An enabled copied database therefore
never takes that path before syncing.

The new Mac can start from a checkpoint owned by the original Mac and use the same `device_id` in
record revisions, conflict ordering, paper-account ownership, and conflict notices. At minimum this
misattributes edits and weakens deterministic conflict handling. More seriously, both machines can
act as one logical writer while one of them skips changes already covered by the copied checkpoint.
The direct unit test of `check_device_identity` does not cover automatic startup.

Recommendation: perform device-identity validation synchronously before any enabled worker starts.
Treat a failure to read or update the identity as a blocking sync error, not as permission to start.
Keep the database's ID-mapping tag only if that is the deliberate lineage rule, but mint a new
device ID, clear checkpoint/in-flight state, and require a fresh catch-up. Add an end-to-end host
test that creates enabled state for hardware A, constructs a host on hardware B through an injected
identity provider, and verifies that the first `Start` command carries no engine state and later
records use a new origin device.

### Medium — A deletion of an unknown future record kind is discarded

Affected: mixed-version use when a newer app introduces a new `EntityKind`.

Unknown records are preserved as intended: `admit` writes them to `sync_inbox` with reason
`unknown_type` (`src/sync/core.rs:747-775`), and a later version retries set-aside records
(`src/sync/apply.rs:1402-1435`). Deletions take a different path. `on_fetched` converts deletion
names with `RecordName::parse` and silently drops names whose prefix this version does not know
(`src/sync/core.rs:696-713`). It does not remove or supersede an existing raw inbox entry by its
opaque record name.

Sequence: a newer Mac creates `futurekind.1`; this build stores it in the inbox; the newer Mac
deletes it; this build discards the unknown deletion and advances its checkpoint; then this Mac is
upgraded to a version that knows `futurekind`. Startup retries and applies the stale inbox payload,
potentially re-uploading data the user deleted.

Recommendation: preserve deletions as opaque record names until their kind is understood, or at
least remove a matching set-aside inbox record immediately without parsing the prefix. A durable
deletion/tombstone inbox shape is safer because it also handles out-of-order deliveries. Add a
mixed-version test covering create, old-client fetch, delete, old-client fetch, upgrade, and
confirmation that the entity remains deleted.

### Medium — A deletion blocked by an unreadable local table is not durably retained

Affected: watchlists, templates, editor themes, portfolios, and any table containing a row from a
newer version that this build cannot decode.

Incoming upserts correctly call `keep(record, "local_unreadable")`, which writes the full remote
record to `sync_inbox` (`src/sync/apply.rs:411-418`). The generic deletion path instead increments
`report.inboxed` and returns without inserting anything (`src/sync/apply.rs:1502-1525`). Since the
batch otherwise succeeds, the covering checkpoint can advance. There is no payload or tombstone to
retry after an update makes the table readable.

The result is a silent divergence: the cloud item remains locally present and may later be
re-uploaded by reconciliation. The report counter is also misleading because no inbox row exists.
Specialized deletion paths for layouts, alerts, scripts, transactions, and paper accounts need the
same audit; several do not use the generic table-writability gate.

Recommendation: represent remote deletions durably in `sync_inbox` (or a dedicated pending-delete
table) with their record name, reason, and receive time. Retry them whenever set-aside records are
retried, and do not count a deletion as inboxed unless persistence succeeded. Add a test with one
unreadable row plus a remote deletion, then simulate an upgraded decoder and verify that the delete
is applied rather than re-uploaded.

### Medium — Pine script file mutations are not atomic with SQLite apply metadata

Affected: remote Pine script updates, deletes, adoption, and conflict copies.

`apply_batch` wraps its work in a SQLite savepoint and is documented as applying one fetched batch
transactionally (`src/sync/apply.rs:84-128`). Scripts are files, however. `apply_script` calls the
real `ScriptStore` to adopt, update, delete, or create a conflict copy before it writes the related
`sync_entity` link and conflict row (`src/sync/apply.rs:556-708`, `711-782`). Those file operations
cannot be rolled back by the SQLite savepoint.

If a later database operation fails, SQLite rolls back while the script file and its metadata files
remain changed. A retry may usually converge, but conflict-copy creation is particularly unsafe:
the copy can be created successfully and `record_conflict` can then fail, causing each retry to
create another copy. A batch containing a script followed by an unrelated failing entity can also
partially affect the script library despite the batch being reported as failed.

Recommendation: split script application into prepare/commit/finalize phases. Stage new files under
unique temporary names, write durable SQLite intent/link metadata, commit the database, then
atomically publish or trash files with a recovery journal for crashes between phases. At minimum,
make operations idempotent with deterministic conflict-copy IDs and detect an already-applied
remote hash. Add injected-failure tests after every script filesystem operation and before every
following SQLite write.

### Medium — Persistence errors at account and destructive boundaries are ignored

Affected: enable/disable, account confirmation and switching, category changes, remote-zone
recovery, cloud-data deletion, and worker startup.

The host discards many `rusqlite::Result`s. The most serious example is after CloudKit confirms
**Delete my iCloud data**: `account::after_cloud_delete` is ignored, the worker is stopped, and the
UI reports success (`src/host/sync_host.rs:418-430`). If the database update fails, local state may
remain enabled and bound. On the next launch the worker can recreate the zone and upload data the
user explicitly deleted. Account bind, enable, disable, category writes, and remote-deletion
resolution have similar ignored results (`src/host/sync_host.rs:478-567`). Account-change failure
also consumes the pending account and stops the worker without surfacing the error
(`src/host/sync_host.rs:591-608`).

The core follows the same pattern for checkpoint saves, acknowledgements, inbox persistence,
outbox state changes, and reconciliation. Some failures are safe because the durable row remains,
but others make the state machine's in-memory state disagree with SQLite. `sync_state.last_error`
exists but runtime errors are not persisted into it.

Recommendation: classify persistence operations as required or best-effort. Required transitions
must return an explicit failure to the state machine, keep syncing paused, and show a durable error;
never report destructive completion until local state is committed. Make checkpoint saving return a
result and leave `pending_state` intact on failure. Add disk-failure tests for account bind,
account switch, checkpoint save, outbox acknowledgement, and post-zone-deletion cleanup.

### Medium — Replacing local collections with iCloud data is not one atomic operation

Affected: first-sync **Use iCloud's data** and account-change **Use This Account's Data**.

`clear_replaceable_collections` deletes five tables in a loop without a surrounding transaction or
savepoint (`src/store/sync.rs:1540-1555`). In `finish_join`, it runs after the backup but before the
transactional remote apply (`src/sync/initial.rs:133-184`). In account-change resolution, sync
metadata is reset before the collections are cleared and the new account is bound
(`src/sync/account.rs:80-126`). A failure partway through can therefore leave a mixture of cleared
and retained collections, partially reset metadata, or an unbound database.

The pre-operation SQLite backup is valuable and prevents irrecoverable loss, but recovery is manual
and the UI does not expose it. The operation's user-visible promise is replacement as one choice,
not a series of independently committed deletes.

Recommendation: place SQLite-only reset, collection clearing, inbound apply, and final sync-state
transition in one transaction where possible. Keep the external backup before that transaction.
For script files, use the recovery protocol recommended above. Add a failure trigger to each table
in turn and assert that either the entire pre-join database remains or the complete cloud choice is
installed, never a partial mix.

### Medium — Generic native send failures have no proven forward-progress path

Affected: manual sends through the Swift bridge when `automaticallySync` is false.

Rust marks every staged outbox row in `inflight` before sending (`src/sync/core.rs:404-463`). Swift
then awaits `engine.sendChanges()` and emits only a generic error if that call throws
(`native/swift/Coordinator.swift:215-220`). `SyncCore::on_error` records the error but does not clear
the matching in-flight entries or issue another `Send` (`src/sync/core.rs:647-670`). Later `pump`
calls skip those rows because they are still busy. A restart clears both sides, but an in-process
retry depends on `CKSyncEngine` also delivering per-record `sentRecordZoneChanges` events or
internally retrying all pending changes.

Apple documents that `sendChanges` returns only after related send events finish and that the
engine handles several transient errors internally. Apple also states that when
`automaticallySync` is false the app must invoke `sendChanges` and `fetchChanges` itself. The current
mock models a wholesale send failure as per-record failures, not as `sendChanges()` throwing before
such events, so it cannot prove this integration contract. See
[CKSyncEngine.sendChanges](https://developer.apple.com/documentation/cloudkit/cksyncengine-5sie5/sendchanges%28_%3A%29)
and
[CKSyncEngine.Configuration.automaticallySync](https://developer.apple.com/documentation/cloudkit/cksyncengine-5sie5/configuration/automaticallysync).

Recommendation: instrument `willSendChanges`, `sentRecordZoneChanges`, and `didSendChanges` on a
real development container while forcing offline, account, service, and cancellation errors.
Extend the bridge test double with a generic throw that emits no per-record event. If that case can
occur, have Swift return the affected staged sequence numbers or have Rust clear all in-flight
markers and explicitly call `sendChanges` again after backoff. Do not remove the durable outbox
entries.

### Low — Fetched batches have no aggregate memory bound

Affected: initial sync and catch-up containing many large pictures or compressed paper-account
records.

Individual payloads are bounded, but `fetchedRecordZoneChanges` maps every modification into a
dictionary array and serializes the whole event as JSON (`native/swift/Coordinator.swift:294-301`,
`native/swift/Codec.swift:57-75`). Binary fields are base64-expanded, then Rust parses the whole
JSON value and clones raw records during admission. A CloudKit batch containing hundreds of records
near the 900 KB wire limit can transiently require hundreds of megabytes or more across Swift data,
base64 text, Serde values, decoded payloads, and apply working copies.

This is bounded by CloudKit's batch and record limits, so it is not unbounded input, but it can be a
practical memory-pressure failure. The documented large-library measurement uses many small records
and one compressed paper account; it does not exercise a near-limit batch.

Recommendation: emit fetched records to Rust in smaller chunks or use a framed binary boundary that
does not base64 every payload into one JSON document. Apply aggregate byte and record caps per event
and retain the checkpoint barrier across all sub-batches. Add a stress test with the maximum number
of near-limit records and record peak resident memory.

### Low — Release documentation contradicts the recorded real-CloudKit results

Affected: release decisions and maintainer understanding.

The opening status in `docs/icloud-sync.md` says the feature has not been run against real CloudKit,
and its Tests section says there are no manual results. The same document's Performance section
describes a real Development-environment run, while `docs/icloud-sync-release-checklist.md` records
successful notarized Production sync, two-Mac operations, account simulations, and large-library
tests through 2026-10-22. The checklist also says the automated suite has 2,982 tests; the current
non-corpus run reports 5,182 passed and 55 ignored.

Recommendation: make the main design document's status derive from the release checklist. State
precisely what has been proven in Development, Production, notarized builds, and simulations, and
retain the unchecked manual scenarios without saying no real results exist. Update test counts only
if they are useful; otherwise avoid a number that becomes stale.

### Low — Several sync-state fields and infrastructure are incomplete or dead

Affected: diagnostics, conflict attribution, and maintenance.

`sync_state.last_error` is read and written by state serialization tests but runtime core errors live
only in memory. `dataset_id` is cleared in recovery paths but never assigned. `sync_device` is
created and queried for a friendly name, but no production code inserts or updates a device row.
Consequently, remote conflict-copy naming normally falls back to "another Mac" rather than the
documented device name (`src/store/sync.rs:93-178`, `1557-1568`; `src/sync/apply.rs:711-729`,
`1071-1094`). The design document calls `sync_device` ready for a future device list, but its current
presence suggests stronger attribution than exists.

Recommendation: either complete the device metadata record/protocol and persist actionable error
state, or remove the unused columns and claims until the behavior is designed. Add schema-level
comments that distinguish reserved fields from active invariants.

## Positive controls and strengths

- The write hooks use the same SQLite savepoint as document/setting changes, and outbox rows are
  sequence-numbered so an older acknowledgement cannot clear a newer edit.
- Uploads are gated on sync enabled, matching account, and completed first-time merge. The bound
  account hash also travels with staged changes for a second check in Swift.
- Projection removes alert runtime state, layout-local fields, and snapshot consent before hashing
  and upload. Secrets and credentials live in local tables or Keychain.
- Validation bounds record size, decompression, JSON depth, floating-point ranges, script names and
  source, layout chart counts, drawings, watchlists, paper rows, and picture identity/content.
- Unknown and newer-schema upserts are retained byte-for-byte, and readable tables are protected
  from rewrite when any local row cannot decode.
- Merge behavior is explicit per entity and has randomized convergence coverage. Script, layout,
  and paper-account conflicts preserve a second copy rather than silently discarding a version.
- The Swift bridge uses the private database and writes content through `encryptedValues`; record
  names and operational metadata remain visible to Apple as documented.
- The current fast suite has broad pure-state-machine, apply, replay, crash, host-thread, portfolio,
  paper-trading, native-codec, and policy coverage.

## Validation performed

- `cargo test --locked`: **5,182 passed, 0 failed, 55 ignored**. The `corpus` feature was not enabled.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- `cargo clippy --all-targets --all-features --locked -- -D warnings`: passed after directing the
  Swift and Clang module caches to writable paths under `/private/tmp`.
- `cargo fmt --all -- --check`: passed.

The review did not run a new signed build, CloudKit schema check, two-Mac scenario, second genuine
Apple Account, push delivery test, sleep/wake test, or fault-injected native `CKSyncEngine` session.
Those remain necessary to validate the bridge-level finding and the unchecked items in
`docs/icloud-sync-release-checklist.md`.

## Recommended order of work

1. Fix failed-apply acknowledgement and add host-level checkpoint fault tests.
2. Enforce portable-setting policy on inbound upserts and add isolation tests.
3. Run device identity validation before automatic worker startup and test copied enabled databases.
4. Make unknown and unreadable deletions durable across upgrades.
5. Stop ignoring required persistence failures, especially after cloud deletion and during account
   transitions.
6. Define a recoverable transaction protocol for script files and make collection replacement
   atomic.
7. Prove or repair the native generic-send retry path with real CloudKit fault injection.
8. Bound aggregate bridge memory and reconcile the release documentation.
