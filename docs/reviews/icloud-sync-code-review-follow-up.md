# iCloud Sync code review follow-up

Date: 2026-11-02  
Reviewed revision: `2bd9ca5` (`main`)  
Original review: [iCloud-sync code review](icloud-sync-code-review.md)  
Review type: static follow-up plus sync-focused automated tests; no implementation changes, signed
build, real-CloudKit fault injection or two-Mac run.

## Executive summary

All three high-severity findings from the original review have been fixed. Across all twelve
findings, eight are fixed, three are partially addressed, and one remains open.

The principal correctness and isolation defects are resolved: failed fetched batches are no longer
acknowledged, inbound settings obey the same portable-key allowlist as outbound settings, and an
enabled database moved to another Mac receives a new device identity before its worker starts.
Unknown and locally blocked deletions are retained safely, required persistence failures are
reported instead of silently ignored, local replacement choices are transactional, and a generic
native send failure now has an explicit retry path.

The remaining implementation gap is the lack of an aggregate memory bound on fetched bridge
batches. Pine script file operations have been made deterministic and replay-safe for the reviewed
failure cases, but they still have no prepare/commit/finalize journal and therefore are not atomic
with SQLite. Reserved sync schema fields and the device table are now described honestly but remain
unused. The release documentation was mostly reconciled, although one stale Developer ID warning
still contradicts the recorded real-hardware result.

## Status summary

| Severity | Finding | Status |
| --- | --- | --- |
| High | Failed remote apply is acknowledged and its checkpoint may advance | Fixed |
| High | Inbound records can overwrite settings classified as local-only | Fixed |
| High | An already-enabled database copied to another Mac keeps the old device identity | Fixed |
| Medium | A deletion of an unknown future record kind is discarded | Fixed |
| Medium | A deletion blocked by an unreadable local table is not durably retained | Fixed |
| Medium | Pine script file mutations are not atomic with SQLite apply metadata | Partially addressed |
| Medium | Persistence errors at account and destructive boundaries are ignored | Fixed |
| Medium | Replacing local collections with iCloud data is not one atomic operation | Fixed |
| Medium | Generic native send failures have no proven forward-progress path | Fixed in code; real-engine fault test still pending |
| Low | Fetched batches have no aggregate memory bound | Open |
| Low | Release documentation contradicts the recorded real-CloudKit results | Partially addressed |
| Low | Several sync-state fields and infrastructure are incomplete or dead | Partially addressed |

## Finding-by-finding verification

### Fixed — A failed remote apply is no longer acknowledged

`Applier::apply_in_order` now stops at the first failed batch, retains that batch and every later
batch in order, applies bounded backoff, and returns only successfully saved batch ids. The
app-thread path sends `WorkerMsg::Applied` only for those ids, and the hidden-window path calls
`remote_applied` only for those ids (`src/host/sync_host.rs`).

Host-level tests cover both paths with an aborting SQLite trigger. They verify that the checkpoint
does not move, later batches do not overtake the failed batch, the error is exposed, and the queued
changes apply after the trigger is removed (`src/host/sync_host/tests.rs`).

### Fixed — Inbound settings obey the local-only policy

`validate` rejects a setting whose key is not portable, `apply_simple` repeats the policy check as a
second guard, and the deletion path ignores remote deletes for local-only settings
(`src/sync/validate.rs`, `src/sync/apply.rs`). Projection and inbound handling both use the centralized
policy helper in `src/sync/policy.rs`.

Tests inject remote upserts and deletions for MCP settings and permissions, AI preferences, update
channel, content consent, and an unknown future key. They verify that the local value and sync
metadata remain unchanged. The separate snapshot-consent merge test still passes.

### Fixed — Copied enabled databases change identity before syncing

`SyncHost::new` checks the current hardware before starting an already-enabled worker and refuses to
start when the check cannot be saved. The application also calls `adopt_this_mac` before loading
state that contains ids (`src/host/sync_host.rs`). Moving the database now mints a new device id,
clears its checkpoint, and rewrites the database's counter-based ids under the appropriate lineage
rules (`src/store/identity.rs`).

Host tests construct an enabled database from hardware A on hardware B and verify the new identity
and cleared checkpoint. Same-hardware, unavailable-identifier, and failed-database-write cases are
also covered.

### Fixed — Deleting an unknown future kind removes its retained copy

Fetched deletion names now remove a matching `unknown_type`, `newer_schema`, or applicable
`initial_pending` inbox row before attempting to parse the record kind (`src/sync/core.rs`). Thus an
older build does not retain an upsert after the newer build has deleted it. The create, retain,
delete sequence has an integration test in `src/sync/apply_tests.rs`.

The documented separate limitation remains: a deletion delivered before the corresponding record
is not represented as a durable opaque tombstone.

### Fixed — Deletions blocked by unreadable tables are durable

The apply layer writes these deletions to `sync_inbox` as `delete_pending`, with the record name and
an empty payload. Every apply retries pending deletions once the affected table becomes writable.
Records retained as `local_unreadable` are likewise reconsidered (`src/sync/apply.rs`).

Tests verify that the local row is not prematurely removed, the pending deletion really exists,
and it is applied rather than re-uploaded after the unreadable row is removed.

### Partially addressed — Pine files remain outside the SQLite transaction

The dangerous conflict-copy case has been hardened. The losing version is written before the
winner, failure to preserve it fails the batch, and its id is deterministically derived from its
contents. Reapplying a failed batch therefore finds the same copy rather than producing duplicates
(`src/sync/apply.rs`). Tests cover failure to create the copy and failure in the later conflict-row
write (`src/sync/script_tests.rs`).

This does not make filesystem mutations atomic with SQLite. A script can still be written or
deleted before a later database operation rolls back. Replay is intended to repair these states,
but there is no durable prepare/commit/finalize journal and no injected-failure coverage after every
filesystem boundary. The original finding is therefore mitigated, not fully closed.

### Fixed — Required persistence failures are no longer swallowed

The host now guards enable, disable, account confirmation and switching, category changes, held
deletion choices, remote-zone recovery, and post-cloud-deletion cleanup. A failed required write
prevents the subsequent state transition and is shown to the user (`src/host/sync_host.rs`).

The core reports database failures as `ErrorKind::Storage`. A failed checkpoint remains pending for
retry, a fetched record that cannot be retained holds the checkpoint for the session, an unreadable
outbox is an error rather than an empty queue, and reconciliation is retried. Upload acknowledgement
and retry bookkeeping remain explicitly best-effort because a failure leaves the durable outbox row
available to send again (`src/sync/core.rs`). Fault tests cover the checkpoint, inbox, outbox,
disable, and post-zone-deletion cases.

### Fixed — Replacing local collections is transactional

First-sync **Use iCloud's data** now clears replaceable collections and applies the fetched records
inside one savepoint (`src/sync/initial.rs`). Account-change choices reset sync metadata, clear the
selected collections, and bind or disable the account inside one savepoint (`src/sync/account.rs`).
The external backup is deliberately created before the transaction and remains available on either
outcome.

Fault tests verify that a failure after clearing has begun restores the complete pre-join database,
and that a failed account bind leaves the old data, metadata, and account binding intact.

### Fixed in code — A wholesale native send failure retries

When `CKSyncEngine.sendChanges()` throws, Swift emits a distinct `send_error` event
(`native/swift/Coordinator.swift`). Rust decodes it as `TransportEvent::SendFailed`, clears the
in-memory in-flight markers while retaining the durable outbox rows, and sends them again after
backoff (`src/sync/core.rs`). The fake native bridge and `MockCloud::Fault::SendThrows` cover event
decoding and forward progress without a restart.

This path has not been observed against a real Development container with offline, account,
service, or cancellation failures. That is a remaining verification task, not a missing retry path.

### Open — Fetched bridge batches still lack an aggregate memory bound

The Swift event handler still maps every `fetchedRecordZoneChanges` modification into one array,
base64-encodes the record payloads and system fields, and serializes the entire event as one JSON
document (`native/swift/Coordinator.swift`, `native/swift/Codec.swift`). No aggregate record or byte
cap, chunking protocol, or near-limit batch memory test was added.

Individual records remain bounded, and CloudKit limits the batch, but the original transient-memory
risk remains.

### Partially addressed — Release documentation still has one contradiction

The opening of `docs/icloud-sync.md` now accurately states which Production and Development runs
were completed, points to the release checklist for results, and no longer claims that real
CloudKit has never been exercised. The stale automated-test count was removed.

However, `docs/icloud-sync.md` under **What is not verified** still says it is unknown whether a
Developer ID profile can carry the required entitlements, notarize, launch, and select the expected
environment. That contradicts the same document's opening and the checked Developer ID result in
`docs/icloud-sync-release-checklist.md`. The documentation finding is therefore not fully closed.

### Partially addressed — Reserved infrastructure is documented but still unused

`sync_state.dataset_id`, `sync_state.last_error`, and `sync_device` are now marked as reserved in
the Rust model and schema comments. The design document also says explicitly that no device list or
cross-device name protocol exists (`src/store/sync.rs`, `docs/icloud-sync.md`). This removes the
misleading implication that the infrastructure is active.

The fields and table remain unused: runtime errors are memory-only, `dataset_id` is never assigned,
and no production path writes `sync_device`. The maintenance concern remains, although its status
and limitations are now clear.

## Validation performed

`cargo test --locked sync::`:

- 459 passed
- 0 failed
- 2 ignored
- 4,823 filtered out

The filter includes the sync engine, apply, crash, account, first-join, script, native-codec,
policy, storage-hook, host-thread and sync settings tests, as well as unrelated chart-sync tests
whose module names match the filter.

No new signed build, CloudKit schema check, real-engine send fault, two-Mac scenario, second genuine
Apple Account, sleep/wake test, or network-drop test was run for this follow-up.

## Remaining work

1. Add an aggregate cap or chunked/framed bridge protocol for fetched records, preserving one
   checkpoint barrier across every chunk, and measure a batch of near-limit records.
2. Decide whether replay-based Pine recovery is sufficient. If not, add a durable
   prepare/commit/finalize filesystem journal and fault tests at every file/database boundary.
3. Remove the stale Developer ID item from **What is not verified** and retain only the genuinely
   unchecked release scenarios.
4. Either design and implement the reserved device/error metadata or keep it explicitly reserved
   until schema compatibility permits removal.
5. Run the real `sendChanges()` fault matrix against a signed Development build and record which
   failures throw wholesale versus producing per-record completion events.
