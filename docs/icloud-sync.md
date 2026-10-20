# iCloud Sync

Keeps watchlists, layouts (with their drawings), chart and drawing templates, Pine scripts, price
alert definitions and a few preferences the same on every Mac signed in to one Apple Account.

**Status.** The sync engine, merge rules, account protection, UI and Swift bridge are written and
tested against an in-memory CloudKit stand-in. **It has not been run against real CloudKit.** Two
things must be proven on real hardware before a release enables it (see
[What is not verified](#what-is-not-verified)): that a Developer ID build can use CloudKit with the
entitlements below, and that CloudKit pushes reach the app. Do not describe the feature as
production-ready until the two-Mac and notarized-build checks below have passed.

SQLite stays the source of truth. CloudKit is a transport: every read and write the app makes is
against SQLite, and every sync problem leaves the app fully usable.

## What syncs

| Syncs | Record | Notes |
| --- | --- | --- |
| Watchlists | one per list; Favorites is one shared record | entries merge individually |
| Saved layouts | one per layout | without drawings, `last_opened`, `autosave` (per Mac) |
| Drawings | one per drawing, keyed `layout.chart.drawing` | only inside saved layouts, never the tab session |
| Chart / drawing templates | one each | built-in presets live in code |
| Pine scripts | one per script, content inline | no drafts, no revisions; unmodified bundled examples are skipped |
| Price alert definitions | one per alert | arrive **switched off** on other Macs, see below |
| Preferences | `appTheme`, `drawing.defaults`, `drawing.prefs`, `watchlist.flags`, `snapshot.prefs` (host, scale, include-trading only) | |

Never synced: market and icon caches, paper-trading accounts, portfolios and ledgers, webhook
endpoints and their Keychain secrets, API keys, MCP clients/tokens/port, Pine alert subscriptions
(they bind device-local chart ids), the open tabs and windows, alert runtime state and history,
snapshot consent and token. Every document table and `setting` key is classified in
`src/sync/policy.rs`, and a test fails when one is added without a decision; unclassified means
local.

## Design

```
SettingsPage ← SyncView ← SyncHost (app thread: apply, actions)   ──mpsc──  worker thread
                                                                          SyncCore (pure state machine)
                                                                              │ SyncTransport
                                                         MockCloud (tests) ───┤
                                                         NativeTransport ─ C ABI ─ Swift CKSyncEngine
```

* **Outbox.** `Store::replace_documents` and `set_setting` queue the upload intent in the *same
  SAVEPOINT* as the write (`src/store/sync.rs`). A change can never commit without it, and a crash
  right after a save still uploads after restart. Rows are projected to sync entities first
  (`src/sync/project.rs`): alert runtime fields, per-Mac layout fields and snapshot consent are
  dropped, drawings are split out of layouts. That equality is what makes the alert engine's
  once-a-second rewrite and the apply of a remote change queue nothing.
* **Core** (`src/sync/core.rs`) has no threads or network. It stages due outbox entries in bounded
  batches tagged with the account they belong to, clears an entry only when the cloud accepted
  that exact `seq`, retries with backoff per failure kind (honouring server retry-after), and
  saves the engine checkpoint only after a fetched batch was applied.
* **Apply** (`src/sync/apply.rs`) writes one fetched batch in one transaction. Each remote row is
  checked against this version's own model first; a table holding a row this version cannot read
  is never rewritten; what cannot be applied is kept in `sync_inbox`, never dropped.
* **Bridge** (`native/swift`, `src/sync/native.rs`) moves records between `CKSyncEngine` and the
  core as JSON over a four-function C interface. It never throws across the boundary and refuses
  to stage data for any account other than the one the data is bound to.
* **Linking.** `build.rs` compiles the Swift only with `--features icloud` and only for macOS
  targets, and links it into the `degenview` binary alone. No file is added to the bundle, so
  the updater's bundle checks are unchanged, and the alert agent never links Swift.

### SQLite tables

`sync_state` (singleton: device identity, account binding, engine checkpoint, category switches),
`sync_entity` (per-entity revision, last-synced hash, CloudKit change tag, tombstone),
`sync_outbox`, `sync_inbox`, `sync_conflict`, `sync_device`, `price_alert_local` (per-Mac
"watch here" flag). All are created by one idempotent `create_sync_schema`; older builds ignore
them, and `reconcile_stored` finds changes an older build made.

### Record schema

Private database, zone `DegenViewZone`, **one record type `DegenViewSyncRecord`** so new kinds of
entity never need a manual schema deploy. `recordName = <kind>.<id>`, deterministic from the
entity's identity, so repeated uploads and downloads hit the same record. Plain fields:
`schemaVersion`, `entityType`, `revision`, `originDevice`, `payloadSize`. Encrypted
(`encryptedValues`): `payload` (JSON), `modifiedAtMs`. A record from a newer schema is kept
untouched in the inbox and applied after an update. Payloads over ~900 KB are blocked and
reported (asset upload is not implemented).

### Identity

Entity ids are nanosecond timestamps (watchlists, layouts, templates, alerts) or random 128-bit
(scripts), so they do not collide across Macs. Chart ids are stable inside a layout; drawing ids
are per chart, so a drawing is keyed `layout.chart.drawing`. When two Macs give different
drawings the same id, the local one is renumbered so both survive. Each Mac's Favorites list has
a different local id, so it syncs as the singleton `watchlist.favorites` and keeps its own id.

## Merge rules

Every rule is symmetric (merge(a, b) = merge(b, a)), so two Macs resolving the same conflict
converge. Randomized tests exercise this. Ordering ties break on `(revision, edit time, device id)`.

| Entity | Rule |
| --- | --- |
| Watchlist | three-way: a market is `source:symbol`, so the same market added on two Macs is one row; edit beats delete; order follows whichever side reordered; concurrent inserts ordered by key |
| Layout | revision-aware; if only cosmetic fields differ, last writer; if charts/columns differ, the loser is kept as a copy named `Name (conflict from <Mac> <date>)` |
| Drawing | per record last writer; a delete never touches other drawings (stacking order is deliberately **not** synced: it would make one deletion look like an edit of every later drawing) |
| Pine script | the loser always survives as a conflict-copy script; text being typed is never overwritten (the update waits until it is saved); a name taken here is suffixed and the suffix is not uploaded |
| Templates, alerts, scalar settings | last writer |
| Object-valued settings | per key three-way |

### Deletion

A delete leaves a tombstone (`sync_entity.deleted`, a small metadata row), so a Mac that was
offline cannot resurrect the item: it learns of the deletion when it returns. Tombstones are not
purged in this version; they cost a few bytes each, and compacting them safely needs the device
list that is not built yet.

**Edit beats delete**: if the stale Mac changed the item, the item comes back everywhere, and a
layout brings its drawings back with it. Deleting at least five items and over half of a kind in
one write is *held* for explicit confirmation instead of propagating. A delete is never sent
before the session has completed a fetch, so an edit made elsewhere is seen first.

iCloud does not replay old deletions to a device whose checkpoint is too old. So a Mac whose
checkpoint expired, or that has not fetched for 60 days, starts from scratch, and once that
catch-up completes, anything it holds that iCloud no longer lists is treated as deleted elsewhere
(removed if unedited, kept and uploaded again if edited). If that would remove more than 20 items
and over a quarter of what is synced, it removes nothing and says so: that looks like a problem,
not a clean-up.

### Alerts

Only the **definition** syncs (condition, asset, frequency, note, webhook endpoint ids, paused or
active). Armed state, previous value, triggered state, history and health never do, so firing
never uploads. Remote changes reach the alert owner (the app or the background agent) through the
same command queue as the UI, never a table write the owner would overwrite. An alert that
arrives from another Mac starts **switched off** on this one (`price_alert_local`), so one alert
notifies from one Mac; the Alerts window offers *Watch on This Mac* / *Stop Watching on This Mac*.
Switching it on re-baselines it so it cannot fire from a stale comparison. Webhook endpoints do
not sync, so a synced alert's endpoint ids simply have no target on another Mac.

## First sync and accounts

1. The user turns sync on; an Apple Account must be signed in. Nothing is read or sent until the
   user agrees to use that account.
2. The first fetch is **held in the inbox** and summarised ("iCloud has 12 watchlists… this Mac
   has…"). The user chooses **Merge** (everything from both), **Use iCloud's data** (this Mac's
   watchlists, layouts and templates are replaced after a recoverable database copy is saved in
   `SyncBackups/`; scripts and alerts are always merged) or **Don't sync**. An empty iCloud never
   deletes local data. The step is idempotent after an interruption.
3. Uploads begin only after that step.

A database is bound to one account (a hash, never the identifier). If a different Apple Account is
signed in, sync **pauses with nothing uploaded** and the user chooses: keep local only, use that
account's data (copy saved first), or merge. Signing out only waits. Staging carries the bound
account's hash and the bridge refuses a mismatch. A database copied to another Mac gets a new
device identity. *Delete my iCloud data* is a separate, typed-confirmation action that only
targets the bound account and forgets the binding only after iCloud confirms; other Macs pause and
offer to upload again.

## Privacy

Data is stored in the user's **private** CloudKit database; the public database is never used.
Payload content uses `CKRecord.encryptedValues`. Record names, entity types, sizes and timestamps
are visible to Apple. Do not claim more than Apple documents for `encryptedValues` (the app makes
no end-to-end claim), and tell users that Pine scripts sync as written, so secrets pasted into
them sync too. Webhook secrets, API keys and MCP credentials are never in a synced record.

## One-time Apple Developer setup

Needed once, by the account holder, before a build can use iCloud:

1. Identifiers → `es.nico.degenview`: enable **iCloud (CloudKit)** and **Push Notifications**.
2. Create the container `iCloud.es.nico.degenview` and assign it to the App ID.
3. Regenerate the **Developer ID provisioning profile** for the app; base64 it into the
   `MACOS_PROFILE_APP` secret. The helper's profile is unchanged (it has no iCloud).
4. For development: an Apple Development certificate and a Mac Development profile (register the
   test Macs) that includes iCloud. Development-signed builds use CloudKit's **Development**
   environment.
5. Run a development-signed build once so CloudKit creates `DegenViewSyncRecord` in the
   Development environment, check the fields in the CloudKit Dashboard, then **Deploy Schema
   Changes to Production**. Repeat after any schema change. Production data is not recoverable
   from a missed deploy, so do this before every release that touches the record.

## Build and release

```
DEGENVIEW_ICLOUD=1 ./tools/bundle.sh          # + the usual DEGENVIEW_CODESIGN_IDENTITY, … variables
tools/verify-icloud.sh target/DegenView.app Production
```

`DEGENVIEW_ICLOUD=1` builds with `--features icloud`, signs the *app* (never the helper) with
`tools/entitlements-app.plist`, and runs `tools/verify-icloud.sh`, which checks the app's
entitlements, the container and environment, that the embedded profile grants them, that the
helper has none, and that no library or machine-specific path was bundled. Without the variable
nothing changes: no Swift is compiled and the entitlements stay as before. The Release workflow
passes the variable when the repository variable `DEGENVIEW_ICLOUD` is `1`; set it only after the
profile above is in `MACOS_PROFILE_APP`. A signed build whose profile does not grant its
entitlements is **killed by macOS at launch**, so ship the first iCloud build as a hyphenated
prerelease (the updater ignores those) and install it on a clean Mac first.

At run time the app reads its own signature before touching CloudKit (calling CloudKit without
the entitlement raises an exception Swift cannot catch): a release build must be signed for
**Production** and a debug build for **Development**; any other combination is "unavailable".
Without `--features icloud` the Settings page is hidden.

## Tests

* Automated, no Apple account needed: `cargo test` (outbox atomicity, echo-proofing, coalescing,
  mass-delete hold, merge convergence over randomized edits, core retry/backoff/state machine,
  two-Mac convergence through a mock cloud, first-sync and account-change flows, crash recovery at
  six points, the wire format against a fake bridge, host end to end with real worker threads).
  `cargo build --features icloud` (macOS) additionally compiles and links the Swift bridge.
* Manual, two Macs on one Apple Account and a second Apple Account, with a Development-signed
  build, then a notarized Developer ID build: **A** enable on Mac 1 then join from Mac 2 (merge and
  use-iCloud); **B** concurrent edits to a watchlist, a layout and a script (conflict copies and
  the conflict notice); **C** edit on both while offline, delete versus edit; **D** sign out,
  switch Apple Account, sign back in; **E** turn off and on, delete iCloud data and watch the
  other Mac; **F** a synced alert arrives switched off, the toggle works, delete propagates; an
  autosave-layout drawing edit appears on the other Mac; the alert agent running with the app
  closed. Also: restart, update in place, wake from sleep, network drop.

Record the results here when they exist. There are none yet.

## What is not verified

* Whether a **Developer ID** profile can carry `icloud-container-identifiers`, `icloud-services`,
  `icloud-container-environment` and `aps-environment` on macOS, notarize, and launch, and which
  CloudKit environment such a build uses. Forum evidence is old and ambiguous. Prove this with a
  throwaway signed and notarized build before relying on anything else here. If it fails, the
  fallback is fetching with manual CloudKit operations instead of `CKSyncEngine`.
* Whether CloudKit pushes reach the app (winit owns the app delegate). The worker also looks for
  changes every five minutes and on **Sync Now**, so the feature degrades to polling.
* Asset (`CKAsset`) upload for records over ~900 KB, and whether assets get the same encryption.
* Behaviour with all windows hidden: remote changes are applied on the app thread, which only runs
  frames while a window is visible; uploads are unaffected.
* Real-world performance numbers (initial sync of a large library, outbox cost) are unmeasured.

## Known limitations

* The first-time merge replaces only watchlists, layouts and templates in *Use iCloud's data*;
  scripts and alerts always merge.
* A delete reaching iCloud before another Mac's concurrent edit of the same item is seen can still
  lose that edit (CloudKit deletes are unconditional); the delete-after-fetch rule narrows the
  window, it cannot close it.
* If another Mac deletes a layout while this one has edited it, the layout returns; but a drawing
  deletion that reached this Mac in an earlier batch than the layout deletion has already removed
  that drawing locally.
* No device list in Settings yet (`sync_device` is ready for it).
* Webhook endpoints, Pine alert subscriptions, paper trading and portfolios are excluded by
  design; syncing the last two needs a separate design with immutable transaction ids.
