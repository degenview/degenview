# iCloud Sync (PR #84): production-readiness checklist

The automated suite (`cargo test`, 2982 tests) uses a mock cloud and a fake bridge. Real CloudKit
has only been exercised on two Macs on one Apple Account with notarized Developer ID builds
(Production), and so far for watchlists, drawings and alerts. Record the result next to each item as
you go.

**Status 2026-10-21.** Working on two real Macs: watchlist add/remove, drawings in an open Autosave
layout, alerts. Fixed on the way (all on the PR): flags category and merge, rows an older build
wrote without queuing (found at start and every 5 min), a layout changed by another Mac now reaches a
tab that has it open, polling every 20 s (pushes arrive only now and then), grouped "changed on two
Macs" notices. **Next:** CI green, then sections 1 and 2 (first sync choices and account safety), the
unexplained paper-trading conflicts, then the rest of section 3.

Test setup: two Macs on one Apple Account, plus a second Apple Account. Use a Development-signed
build first, then a notarized Developer ID build.

## 0. Blockers: prove these first

- [x] **Developer ID signing.** Build a throwaway signed and notarized build
  (`DEGENVIEW_ICLOUD=1 ./tools/bundle.sh`, then `tools/verify-icloud.sh target/DegenView.app Production`).
  It must launch (not be killed by macOS), the entitlements and embedded profile must match, and it must
  use the Production CloudKit environment. If this fails, the design needs a different fetch path.
  *Done 2026-10-21: notarized builds launch and sync on two Macs in Production.*
- [x] **Apple setup.** iCloud (CloudKit) and Push Notifications enabled on the `es.nico.degenview` App ID.
  Container `iCloud.es.nico.degenview` exists and is assigned. `MACOS_PROFILE_APP` regenerated.
- [x] **CloudKit schema.** *(Checked directly 2026-10-22 with `tools/check-cloudkit-schema.sh`: Production
  and Development schemas are identical and Production has the sync record and its five plain fields.
  Re-run it before any release that changes the record and after every deploy.)*
- [ ] **Release path.** First build is a hyphenated prerelease (the updater ignores those). Install it on a
  clean Mac first. Repo variable `DEGENVIEW_ICLOUD` stays off until the profile is in place.
- [ ] **CI.** macOS and Linux checks green on the PR. *(Both were still running after the force-push of
  `cfdea06` on 2026-10-21; the PR merges cleanly.)*

## 1. First sync and joining

- [x] Production smoke test with the notarized build from `d9df54a` on two Macs (2026-10-22): both relaunched, sync works both ways.

- [x] A1: enable sync on Mac 1. Nothing uploads until the account is agreed to. *(Enabled and uploading
  since 2026-10-20; the "nothing uploads before agreeing" part was not watched.)*
- [x] A2: join from Mac 2 with **Merge**. Both Macs end up with the union. *(Verified 2026-10-21 in the
  Development environment with sandboxed instances: a fresh Mac kept its own list and gained iCloud's
  3 watchlists, 5 layouts, 15 scripts, 1 alert and 2 paper accounts; everything it had was uploaded.)*
- [x] A3: join with **Use iCloud's data**. Local watchlists, layouts and templates are replaced and a copy
  is saved in `SyncBackups/`. *(Verified: replaced, nothing of this Mac uploaded, iCloud untouched,
  1.1 MB backup passes `integrity_check` and still holds the old lists. There is no in-app restore:
  it means copying the file back by hand.)* Scripts, alerts, portfolios and paper accounts still merge.
- [x] A4: join with **Don't sync**. Nothing changes, nothing uploads. *(Verified: sync off, held records
  cleared, local lists intact, nothing uploaded.)*
- [x] A5: enable against an empty iCloud. No local data is deleted. *(Verified: "iCloud has no DegenView
  data yet", Start Syncing uploaded the Mac's lists, nothing local changed.)*
- [x] Quit the app mid-first-sync and restart. The step is idempotent. *(Restarted at the join prompt
  several times: the prompt came back and nothing was applied or lost.)*

## 2. Accounts and safety

- [x] D: sign in with a different Apple Account. Sync pauses with nothing uploaded. Each choice works:
  keep local only, use that account's data (copy saved first), merge. *(Simulated in the Development
  environment: three clones of a synced Mac each had their stored account rewritten to a different
  hash, so the signed-in account looked different. All three paused with "A different Apple Account is
  signed in. Nothing from this Mac has been sent to it." and sent nothing. Keep Local Only turned sync
  off and kept the data. Use This Account's Data saved a backup that passes `integrity_check` and
  still holds the Mac's own list, replaced the watchlists and templates and then asked the usual
  merge question. Merge Into This Account asked the same question and then uploaded: only that
  Mac's list ever reached the account. Not covered: a genuine second Apple Account and CloudKit's
  real sign-out and sign-in events.)*
- [ ] D: sign out, then sign back in with the original account.
- [ ] Copy the database to another Mac. It gets a new device identity.
- [x] E: turn sync off and on. *(Real CloudKit, Development: with sync off on X, X added a list, flipped
  the theme, deleted its drawing templates and edited a script while Y added a list; nothing crossed in
  either direction. Turning it back on needed no prompt, X received Y's list and uploaded its own
  changes within seconds, and Y received all of X's (list, deletions, theme, script). No new conflicts
  on either Mac.)*
- [x] E: *Delete my iCloud data* (typed confirmation). The other Mac pauses and offers to upload again.
  *(Verified: the red button does nothing until "delete" is typed; afterwards sync is off, the binding
  and tracking are forgotten and local data is kept; the other running Mac showed "Paused: iCloud
  data was removed" with Upload This Mac's Data Again / Turn Sync Off. Uploading again met the data
  another Mac had put there and offered the normal join.)*
- [x] Mass delete: delete at least 5 items and over half of a kind. The deletes are held. *Confirm* and
  *Keep* both work. *(Verified with 5 layouts: held, nothing reached iCloud; Keep on Other Macs kept
  them for the other Macs and did not re-queue on the next Sync Now; Delete Everywhere sent all 5
  deletions. The two buttons sit tight against the next heading.)*

## 3. Per-feature sync (test A to B and B to A)

### Watchlists and flags
- [x] Adding and removing items sync (2026-10-21).
- [ ] Order of items and lists syncs.
- [x] Flags follow the Watchlists switch, not Preferences. *(Real CloudKit, Development: flags from two Macs merged to {BTC: Red, ETH: Blue}; removing one on a Mac removed it on the other and it did not come back.)*
- [x] Flags on different markets set from two Macs are both kept.
- [x] A flag removed on one Mac does not come back.

### Layouts and drawings
- [ ] Drawings sync only through a saved layout. Test with Autosave on, Autosave off, and a plain tab
  session (this last one should not sync).
- [x] A drawing made on one Mac appears on the other with the layout open and Autosave on (2026-10-21, after the open-layout fix).
- [ ] Move and delete a drawing. A delete never touches other drawings.
- [ ] Two Macs give a drawing the same id. Both survive.
- [ ] `last_opened` and the per-Mac autosave state are not overwritten by the other Mac.

### Templates, scripts, preferences
- [x] Chart and drawing templates: create, rename, delete. *(Real CloudKit, Development.)*
- [x] Pine scripts: edit, delete, and the same script edited on both Macs at once (kept copy named "conflict from" plus a notice). *(Real CloudKit, Development. File rename not tried.)*
- [x] Theme syncs (last writer wins). *(Drawing defaults not tried.)*

### Alerts (F)
- [x] An alert made on one Mac reaches the other (2026-10-21; was slow until the 20 s poll).
- [ ] A synced alert arrives switched off.
- [ ] *Watch on This Mac* and *Stop Watching on This Mac* work.
- [ ] Delete propagates.
- [ ] Exactly one Mac notifies.
- [ ] Repeat with the alert agent running and the app closed.

### Portfolios
- [x] Transactions added on both Macs merge. *(Two Macs both had a "portfolio 1" and "transaction 1": both portfolios and all 5 transactions arrived on each Mac under the right portfolio.)*
- [x] Deleting a portfolio takes its ledger, unless it was edited on the other Mac or a transaction was added there at the same time. *(Fixed 2026-10-21 and verified on real CloudKit in Development: Y deleted a portfolio, X added a transaction a few seconds later; both Macs ended with the portfolio and both transactions. A transaction this Mac uploaded more than 30 s before the delete arrives still goes with the portfolio.)*
- [x] A sale whose purchase has not arrived is held as `ledger_invalid` with a notice. *(Verified with an impossible sale of 500 against 60 held. The notice does not name the portfolio, and it stays after that portfolio is deleted.)*
- [ ] Portfolio cards follow the right portfolio on the other Mac.

### Paper trading
- [x] Place, cancel and modify an order on Mac 1. Mac 2's mirror updates (allow the 5 s debounce). *(Real CloudKit, Development, driven in the app: a limit buy placed on X reached Y as a working order with the "Running on another Mac" banner; cancelling it on Y took the account over and X became the mirror; a new order on X took it back; dragging the order's line on X changed its limit and Y showed the new price and margin; a market buy on Y filled and X received the fill and position. Each change arrived within about 5 to 20 s, including on the Mac whose window was covered.)*
- [ ] Acting on a mirrored account takes it over.
- [x] Both Macs change an account before seeing each other: a paused "(conflict from ...)" copy appears. *(Real CloudKit, Development: both Macs ended with the account plus "Paper Trading (conflict from ...)" and a notice on one; no further conflicts followed.)*
- [x] The 7 notices on the other Mac: its history shows 6 `ledger` notices, all created in the same
  second (Oct 21 19:57:54), none open. Cause: a large import arriving in several batches held sales
  whose purchase came later; they applied but the notice stayed. Fixed 2026-10-21 (the notice closes
  when the held record applies). *(Confirmed on that Mac: all 274 transactions are present and `sync_inbox` is empty.)*
- [ ] Only the owning Mac fills working orders.
- [x] A fresh Mac creates its own default paper account, which then shows as a duplicate. *(Fixed 2026-10-21 and verified on real CloudKit in Development: an untouched default is dropped at the first-time merge when iCloud has paper accounts; a default with an order or a changed balance is kept.)* *(Reproduced: a Mac that opened the paper panel before joining kept its own default and every Mac ended with a third "Paper Trading". A Mac that joins first and opens the panel after gets the synced account and creates none.)*

## 4. Offline, conflicts and resilience

- [ ] B: edit the same watchlist, layout and script on both Macs at once. Conflict copies and the notice
  appear.
- [ ] The notices are grouped by kind ("7 paper trading accounts"), **Dismiss** and **Dismiss All** work,
  and the copies can be deleted. *(Checked visually with fake data only.)*
- [ ] Draw different things in the same open layout on both Macs at the same time. Both Macs end up with
  all of them (three-way merge; local wins a tie).
- [ ] Draw on Mac A while the layout is open but unsaved on Mac B. Nothing is lost on either.
- [x] C: edit on both Macs while offline, then reconnect. *(Real CloudKit, Development, sync off on both:
  both added different markets to one list; one deleted a list the other edited and one deleted a list
  the other renamed (both survived, edited); both recoloured the same flag and each added another (flags
  union, the later change wins the shared one); both renamed one template and set the theme
  differently (the later edit wins). Both Macs converged with no conflicts. Found a bug: a flag one
  Mac cleared earlier and the other then set again was dropped, because a Mac's own upload did not
  refresh its stored ancestor for three-way merges. Fixed 2026-10-21 and re-verified: both Macs end
  with the re-set flag.)*
- [x] C: delete on one Mac against edit on the other. *(Lists, as above: the edit wins and the item stays on both Macs.)*
- [x] Drop the network mid-upload. Nothing is lost or duplicated. *(Simulated by freezing `cloudd` for 40 s with 1,305 of 2,005 changes pending: no error, no loss, resumed and drained to 0, a second Mac matched exactly. A genuine network drop was not tried; it needs the Ethernet service turned off.)*
- [ ] Sleep and wake the Mac.
- [x] Restart the app with a pending outbox. *(Real CloudKit, Development: with the app closed, 2,000 transactions and 5 watchlists were edited; on launch 2,005 changes queued, the app was killed with `kill -9` at 1,505 still pending, and relaunched. The outbox went from 1,505 to 0 in about 40 s with no error and no conflict. A fresh Mac then joined: 5,000 transactions with exactly the 2,000 edits, 5 renamed lists, 5,632 records (none duplicated), the same as the first Mac.)*
- [x] Update in place from the previous release, using a real existing database (the sync tables are added
  by guarded migrations). *(Verified with v0.5.0, the previous release, in a sandboxed home: the data
  it had made (layout with 5 drawings, alert, watchlists, portfolio with transactions, both template
  kinds, a script, settings) was byte-identical after the current build opened it, apart from a
  watchlist re-saved compactly and the alert engine's own counters. The sync tables were created. With
  sync enabled the join prompt listed that data correctly, Merge uploaded all of it (layout drawings,
  alert, portfolio next to the tagged ones) with no conflicts, and a row written without tracking was
  found and uploaded by the next sync. Older builds are not required to open the upgraded data.)*
- [x] All windows hidden: remote changes apply even while the window is covered. *(Verified in the
  Development environment: a list made on one instance reached a covered instance within one poll.
  Before the fix it arrived only when the window came to the front.)*
- [ ] Push wake-up: note whether CloudKit pushes reach the app. *(Seen on 2026-10-21: only 2 pushes in 2
  hours, so the app now polls every 20 s.)* *Sync Now* works either way. Record the real latency after
  the 20 s poll, with the receiving window in front and with it hidden or covered.

## 5. Mixed versions

- [x] With the sync build running next to an older installed app and its alert agent on the same database,
  changes the old programs made were never synced. Fixed 2026-10-21: the engine now looks for them when
  sync starts and every 5 minutes. Still replace the old agent together with the app when installing.
- [x] The bundle's login-item alert agent is the sync-aware build (verified in the built bundle).
- [x] After quitting the old programs and starting the new build, stale alerts and deletions reach the other
  Mac. *(Queued and uploaded on this Mac at 19:57; the other Mac received alerts afterwards.)*
- [ ] Install over an existing older install (the real upgrade path), not just run next to it.

## 6. Privacy and security

- [ ] Webhook secrets, API keys, MCP tokens and Keychain items appear in no record. Spot-check payloads in
  `sync_entity`.
- [ ] Only the private CloudKit database is used.
- [ ] Settings copy says Pine scripts sync as written, so pasted secrets sync too.
- [x] A payload over about 900 KB shows the "too large" notice and does not crash (unit tests: only a payload still too big after deflating is blocked). A 1.6 MB-as-written paper account synced deflated on real CloudKit and arrived complete; a 3.9 MB incompressible one was blocked with the notice and nothing else was held up. Capacity: about 5,000 fills (24 MB unpack limit).

## 7. Performance on a real library

- [x] Initial sync of a large library over a real network: 5,632 records up in 155 s, down in 48 s, applied in 6 s.
- [x] Idle CPU and memory of the sync worker: 2% and about 144 MB after the upload (peak 64% / 198 MB up, 82% / 349 MB applying).
- [ ] CloudKit operation counts.

## 8. Record results

- [ ] Write the results into the "Tests" section of `docs/icloud-sync.md`, which currently says there are
  none.
