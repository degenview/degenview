# iCloud Sync

Keeps watchlists, layouts (with their drawings), chart and drawing templates, Pine scripts, price
alert definitions, portfolios with their transactions, paper-trading accounts and a few
preferences the same on every Mac signed in to one Apple Account.

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
| Watchlist flags | one record, `watchlist.flags` | colour marks per market; follows the Watchlists switch and merges key by key |
| Saved layouts | one per layout | without drawings, `last_opened`, `autosave` (per Mac) |
| Drawings | one per drawing, keyed `layout.chart.drawing` | only inside saved layouts, never the tab session |
| Pictures | one per distinct picture, keyed by the hash of its bytes (`drawingasset`) | see [Pictures](#pictures); never deleted |
| Chart / drawing templates | one each | built-in presets live in code |
| Pine scripts | one per script, content inline | no drafts, no revisions; unmodified bundled examples are skipped |
| Price alert definitions | one per alert | arrive **switched off** on other Macs, see below |
| Portfolios | one per portfolio, and one per transaction | valuation history is not synced; it is rebuilt on each Mac |
| Paper trading | one record per account holding the account, its orders, fills, positions, events, closed trades and journal, and which Mac runs it | see [Portfolios and paper trading](#portfolios-and-paper-trading) |
| Preferences | `appTheme`, `drawing.defaults`, `drawing.prefs`, `snapshot.prefs` (host, scale, include-trading only) | |

Never synced: market and icon caches, portfolio valuation history, which paper account or
portfolio is selected, live paper quotes, webhook endpoints and their Keychain secrets, API keys, MCP clients/tokens/port, Pine alert subscriptions
(they bind device-local chart ids), the open tabs and windows, alert runtime state and history,
snapshot consent and token, and `content.prefs` (whether a Post card may fetch a preview from X). Every document table and `setting` key is classified in
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
"watch here" flag), `paper_account_local` (which Mac runs each paper account; absent means this
one). `sync_state.id_tag` and `sync_entity.local_hash` were added to older development databases
by a guarded `ALTER`. All are created by one idempotent `create_sync_schema`; older builds ignore
them, and `reconcile_stored` finds changes an older build (or an older alert agent) made: when
sync starts, every five minutes and on **Sync Now**.

### Record schema

Private database, zone `DegenViewZone`, **one record type `DegenViewSyncRecord`** so new kinds of
entity never need a manual schema deploy. `recordName = <kind>.<id>`, deterministic from the
entity's identity, so repeated uploads and downloads hit the same record. Plain fields:
`schemaVersion`, `entityType`, `revision`, `originDevice`, `payloadSize`. Encrypted
(`encryptedValues`): `payload` (JSON), `modifiedAtMs`. A record this build cannot read when it
arrives (a kind it does not know, `unknown_type`, or a schema above its own, `newer_schema`) is kept
untouched in the inbox. An update that teaches it to read the record applies it: `apply_batch` looks at
those entries again on every apply, and a launch with any waiting (after the first sync) asks for one
empty apply, so nothing waits for new changes. What it still cannot read stays put, byte for byte. See
[Newer versions](#newer-versions). A payload over ~900 KB as written (in practice a long paper-trading
history) is deflated behind a `DVZ1` marker (`record::to_wire` / `from_wire`, applied in the core's
`build` and `RawRecord::into_record`, so nothing else sees the difference); a paper account
built by the real engine shrinks about 30 times. A payload that is still over ~900 KB deflated is blocked and
reported (asset upload is not implemented). A deflated record cannot unpack past 24 MB.

### Identity

Entity ids are nanosecond timestamps (watchlists, layouts, templates, alerts) or random 128-bit
(scripts), so they do not collide across Macs. Chart ids are stable inside a layout; drawing ids
are per chart, so a drawing is keyed `layout.chart.drawing`. When two Macs give different
drawings the same id, the local one is renumbered so both survive. Each Mac's Favorites list has
a different local id, so it syncs as the singleton `watchlist.favorites` and keeps its own id.

## Pictures

An Image drawing names its picture by the sha256 of the stored bytes and carries nothing else, so the
drawing record stays small and a layout never embeds pixels. The picture is its own record
(`EntityKind::DrawingAsset`, prefix `drawingasset`, category Layouts, table `drawing_asset`),
`{ id, mime, width, height, data }` with the bytes base64 in `data`.

* **Limits.** A stored picture is at most 600 000 bytes (`image_ingest` re-encodes and shrinks until it
  fits), so its record stays under `MAX_PAYLOAD_BYTES` (900 000) after base64. No `CKAsset` is used.
* **Validation** (`sync/validate.rs`): a record is accepted only if its name is 64 lower-case hex
  digits, the mime is PNG or JPEG, the bytes are within the limit, start with that format's signature
  and hash to the name. A record that is anything else is refused, not stored.
* **Merge.** A picture is immutable (its name is its hash), so there is nothing to merge: last writer
  wins is moot, and a copy that arrives is the copy. It is reconciled by name only; the picture table is
  not read for fingerprints.
* **Order.** A drawing may arrive before its picture. It is stored as usual and paints a placeholder
  ("Image unavailable (waiting to sync)") until the picture arrives, which the host asks the store for
  every two seconds while it is visible.
* **No deletion.** A picture is never deleted or tombstoned, locally or remotely: layouts sync but tab
  sessions do not, so one Mac cannot know another still draws a picture, and a remote delete is ignored
  (and the row sent again). Deleting a drawing, undo, replace and deleting a layout all leave the
  picture. Cost: at most 600 kB per distinct picture, for good. A scan across devices before deleting is
  the way to fix this; it is not built.
* **What uses a picture:** Image drawings, and the author's picture on a fetched Post card (named in
  its preview). The same rules apply to both: stored once by hash, synced as their own record, never
  deleted.
* **Writes** go through `Store::put_asset` (idempotent, one outbox entry in its own savepoint); a picture
  that arrived from iCloud is written with `put_asset_from_cloud` (no entry back).

Tested on `MockCloud` (`sync/asset_tests.rs`: upload, a second device receiving it, a picture arriving
after its drawing, a remote delete, a layout that stops using a picture, and all twelve text, note and
content drawings reaching the other Mac exactly as drawn; `sync/validate.rs`: damaged and oversize
records). Not verified: a real CloudKit transfer of a
picture, and a record near the size limit.

## Newer versions

What an older build does with data a newer one wrote, and the rules a change has to follow so that
stays safe. (Read from the code; it has not been tried between two real builds.)

| A newer build writes | An older build |
|---|---|
| A new drawing tool (a new `Geometry` variant in the existing drawing record, same schema number) | Accepts the record and stores it. The drawing decodes as `Drawing::Unreadable`: it is not drawn, selected or edited, is listed in the Object tree as "From a newer version" (deletable, nothing else), and is written back and synced byte for byte. |
| A new field on an existing tool, marked `"since": N` above the older build's `DRAWING_FORMAT` | The same: kept whole as `Unreadable`, never re-saved without the field. |
| A new field on an existing tool **without** the marker | The older build decodes the drawing, ignores the field and re-saves it without it, and the stripped copy syncs back over the newer one. Not allowed: see the rule below. |
| A new record kind (as pictures were), or a higher `SCHEMA_VERSION` | Kept in the inbox, applied by the first launch that reads it (see above). |
| A new tool or magnet value in `drawing.prefs` | That one option falls back to its default; the others are kept. (The fallback is then what this build saves.) |

The rule: **a new tool is a new `Geometry` variant. A change to an existing tool's stored form that an
older build would lose (a new field it ignores) must write `"since": N` in the drawing's body, with
`DRAWING_FORMAT` in `model/drawings.rs` raised to N.** A field that only some drawings use can be left
out of the row while it holds its default (as the style fields are) and needs no marker until it is
set. Nothing tells a person on the older build that a drawing is hidden except the Object tree row;
a count elsewhere would be easy to add.

## Portfolios and paper trading

**Ids.** Portfolios, transactions and everything in a paper account are numbered 1, 2, 3… by each
Mac on its own, so every Mac has a "portfolio 1" and an "order 1". Nothing local is renumbered.
At the sync boundary a counter id below 2^24 is tagged with a random 31-bit number stored once in
`sync_state.id_tag` (`2^55 | tag << 24 | id`), and the tag is stripped again when this database's
own record comes back (`src/sync/ids.rs`, a pure function with no table to lose). Ids at or above
2^24 (nanosecond ids, other Macs' tagged ids) pass through. The engines mint counters from ids
below 2^24 only, so a record that arrived from another Mac never moves the counter into the shared
range. Portfolio cards in layouts and watchlists (`card.portfolio_id` and the `portfolio:<kind>:<id>`
key) are translated the same way, so a card follows its portfolio and never points at a different
one on another Mac.

**Portfolios.** A portfolio and each transaction are their own record, merged by last writer, so
adding or editing transactions on two Macs merges. A deleted portfolio takes its transactions with
it unless the portfolio was edited here, or a transaction was added or changed here that iCloud
has not confirmed yet or that this Mac uploaded in the last 30 seconds (edit beats delete, and it
keeps its ledger). Older transactions go with the portfolio. A transaction whose portfolio has not
arrived waits in the inbox as `orphan`.

A merged ledger can be impossible: a sale whose purchase has not arrived, or a purchase deleted
elsewhere that a sale needs. Each batch is checked once per portfolio with the portfolio screen's
own accounting (`portfolio_accounting::holdings`). A transaction that would break it is **held in
the inbox** as `ledger_invalid` with a notice, and tried again whenever more arrives (the notice
closes by itself once the transaction applies, which is routine during a large import that arrives
in several batches); a delete that would break it is not applied and the transaction goes back to iCloud. The
order records arrive in does not matter. Valuation history after any changed transaction is
dropped and marked stale so it is rebuilt.

**Paper accounts: one active Mac.** An account is a live state machine (working orders fill from
quotes), so it is never merged field by field. The Mac that last placed or changed an order
**owns** it (`owner` in the bundle) and is the only one that fills its working orders; the others
show a mirror and `PaperHost` skips those accounts when quotes arrive. Acting on a mirrored
account (placing, cancelling or modifying an order, changing settings, resetting, deleting) takes
it over in the same save. A burst of fills uploads once, five seconds after the first change.
If both Macs changed an account before seeing each other, the later edit keeps the account and the
other version survives whole as a **paused copy** (`Name (conflict from <Mac> <date>)`) with ids
of its own and no owner, so nothing runs it until the user acts on it.

An account whose bundle is over ~900 KB as written is deflated to fit one record. Measured with
round trips made by the engine (a limit buy and a limit sell, both filled), one round trip is about
9.2 KB as written and 0.3 KB deflated, growing linearly (checked to 400 round trips, 800 fills). The
24 MB unpack limit therefore binds first, at about 2,600 round trips (some 5,000 fills); the 900 KB
wire limit would come at about 7,600. One that is still too large is blocked (the "too large" notice
applies) and stays on its Mac, and multi-record accounts are future work. A mirror shows the account as of the owner's last upload.

## Merge rules

Every rule is symmetric (merge(a, b) = merge(b, a)), so two Macs resolving the same conflict
converge. Randomized tests exercise this. Ordering ties break on `(revision, edit time, device id)`.

| Entity | Rule |
| --- | --- |
| Watchlist | three-way: a market is `source:symbol`, so the same market added on two Macs is one row; edit beats delete; order follows whichever side reordered; concurrent inserts ordered by key |
| Layout | revision-aware; if only cosmetic fields differ, last writer; if charts/columns differ, the loser is kept as a copy named `Name (conflict from <Mac> <date>)` |
| Drawing | per record last writer; a delete never touches other drawings (stacking order is deliberately **not** synced: it would make one deletion look like an edit of every later drawing) |
| Pine script | the loser always survives as a conflict-copy script; text being typed is never overwritten (the update waits until it is saved); a name taken here is suffixed and the suffix is not uploaded |
| Portfolio, transaction | last writer; a change that would make the ledger impossible is held (see above) |
| Paper account | the changed side wins; both changed: later edit keeps the account, the other becomes a paused copy |
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

When two Macs upload before either has fetched, the cloud refuses the second upload. For
watchlists, settings, layouts, scripts and paper accounts that refusal is not resolved by saving the
newer edit over the other: the cloud's version goes to the same merge as a fetched one, the edit
stays queued, and the result uploads once it has been applied (five attempts at most, then the newer
side is kept). Drawings, templates, alerts, portfolios and transactions are last-writer-wins, so for
them saving the newer edit is the merge.

## First sync and accounts

1. The user turns sync on; an Apple Account must be signed in. Nothing is read or sent until the
   user agrees to use that account.
2. The first fetch is **held in the inbox** and summarised ("iCloud has 12 watchlists… this Mac
   has…"). The user chooses **Merge** (everything from both), **Use iCloud's data** (this Mac's
   watchlists, layouts and templates are replaced after a recoverable database copy is saved in
   `SyncBackups/`; scripts, alerts, portfolios and paper accounts are always merged) or **Don't sync**. An empty iCloud never
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
6. Check it: `tools/check-cloudkit-schema.sh` exports the Development and Production schemas with
   `xcrun cktool`, fails if Production lacks `DegenViewSyncRecord` or one of the five plain fields the
   bridge writes (`schemaVersion`, `entityType`, `revision`, `originDevice`, `payloadSize`; the
   payload and `modifiedAtMs` are encrypted values with no schema entry), and fails if the two
   schemas differ. It needs a CloudKit management token saved once with `xcrun cktool save-token`
   (CloudKit Dashboard, Settings, Tokens & Keys). Run it before every release and after every deploy.
   Without a token, a Production build that uploads a field that was never deployed has every upload
   of that field refused, so also keep an eye on the "Something went wrong syncing" status after the
   first release build is installed.

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
  six points, portfolios and paper accounts across two Macs with colliding small ids, the wire format against a fake bridge, host end to end with real worker threads).
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

## Performance

Measured in a release build on an Apple Silicon Mac with an in-memory database
(`cargo test --release --lib sync::perf_tests -- --ignored --nocapture`; ignored by default as
timings are machine-specific). The write hook only projects rows whose text changed.

| Operation | Sync off | Sync on | Added |
| --- | --- | --- | --- |
| Save 30 layouts × 20 drawings, nothing changed | 0.55 ms | 0.54 ms | ~0 |
| …one drawing edited | 0.55 ms | 0.79 ms | 0.25 ms |
| Save 20 watchlists × 50 entries, nothing changed | 0.77 ms | 0.74 ms | ~0 |
| Alert engine rewrites 100 alerts (every row's runtime fields differ) | 0.32 ms | 1.54 ms | 1.2 ms, about once a second while alerts run |
| Apply 200 fetched records in one transaction | | 5 ms | |
| Reconcile 30 layouts (600 drawings) + 20 watchlists | | 7 ms | |
| Save a 5,000-transaction ledger, nothing changed | 7.8 ms | 14.6 ms | 6.9 ms |
| Save a paper account with 2,000 fills, nothing changed | 3.0 ms | 16.5 ms | 13.5 ms (the bundle is rebuilt and hashed on every save; a quote only saves while a working order exists for it) |

Nothing here runs on the render thread except the SQLite write hook itself, and no CloudKit call
is ever made from it.

A large library over real CloudKit (Development environment, a Development-signed release build on one Mac Studio): 5,632 records (26 watchlists, 12 layouts with 300 drawings, 80 templates, 60 scripts,
150 alerts, 3 portfolios with 5,000 transactions, one paper account whose journal is 1.6 MB as written).

| Step | Time | CPU (peak / idle) | Memory (peak / idle) |
| --- | --- | --- | --- |
| First upload from the Mac that has the data | 155 s (about 36 records a second) | 64% / 2% | 198 MB / 144 MB |
| Download on a second, empty Mac | 48 s | 40% | 218 MB |
| Apply them (*Use iCloud's data*) | 6 s | 82% | 349 MB peak, 130 MB after |

The paper account uploaded deflated and arrived complete: 2,500 journal rows, the same text. The
upload is limited by CloudKit (batches of about 200 records), not by this app. Not measured: CloudKit operation counts, and a slow or interrupted network.

One slowness this run found is **not** sync's: every frame of the main window reads and parses all saved
layouts (`Workspace::show_toolbar` calls `load_saved_views`). With 8 MB of layouts that takes 60 ms a
frame, and the app sat at 90% CPU with sync off; the author's own layouts (46 KB) cost about 0.3 ms.
It is on `main` too.

## What is not verified

* Whether a **Developer ID** profile can carry `icloud-container-identifiers`, `icloud-services`,
  `icloud-container-environment` and `aps-environment` on macOS, notarize, and launch, and which
  CloudKit environment such a build uses. Forum evidence is old and ambiguous. Prove this with a
  throwaway signed and notarized build before relying on anything else here. If it fails, the
  fallback is fetching with manual CloudKit operations instead of `CKSyncEngine`.
* Whether CloudKit pushes reach the app (winit owns the app delegate). Measured on a Developer ID
  build: rarely. The engine runs with `automaticallySync = false` because, with it on, a covered or
  background app's `fetchChanges()` returns at once without a network call (the engine hands it to
  the system's background scheduler). The worker asks for changes itself every 20 seconds (and
  scans for rows an older build wrote without queuing them every five minutes) and on **Sync Now**,
  so a change reaches a covered window in about 20 seconds. Pushes are not relied on at all.
* Asset (`CKAsset`) upload for records over ~900 KB, and whether assets get the same encryption.
* Pictures between two real Macs ([Pictures](#pictures)): only `MockCloud` has carried one.
* Behaviour with all windows hidden: remote changes are applied on the app thread while windows
  draw. When it has not ticked for 3 seconds (a covered or hidden window draws nothing) the sync
  worker applies them itself and the app reloads what it holds from the reports on its first frame
  back. Measured with a covered instance: a change arrived within one 20-second poll.

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
* Webhook endpoints and Pine alert subscriptions are excluded by design.
* A paper account over the limit above (about 5,000 fills) does not sync yet. Two Macs acting on one account within a sync
  interval produce a paused copy rather than a merge. A Mac that made its own default paper account
  and had *used* it (an order, a changed balance) before joining keeps it next to the synced ones;
  an untouched default is dropped at the first-time merge when iCloud has paper accounts. A card
  pointing at a portfolio that has not arrived yet shows All Portfolios until it does.
