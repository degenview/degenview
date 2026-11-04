# Paper Trading code review

Date: 2026-11-04  
Reviewed revision: `8b76f8e` (`main`)  
Scope: paper account and instrument models, order-ticket validation, execution and accounting,
chart quote routing, the host/persistence boundary, iCloud ownership, chart overlays, panel UI and
paper-focused automated tests.  
Review type: static review plus focused automated checks; no implementation changes, live-provider
fault injection, long-running market session or manual GUI pass.

## Executive summary

The paper-trading implementation has a good structural foundation. The execution engine is pure,
the host builds candidate snapshots and publishes them only after SQLite succeeds, rejected orders
are retained for audit, account state is stored transactionally, and iCloud treats an account as a
single owned state machine rather than merging fills field by field. The ticket and formatting code
also have useful focused tests, and strict store reads avoid silently dropping an undecodable order
or fill.

It is not production-ready yet. I found four high-severity correctness defects. First, working
orders are driven by chart rendering: closing the Paper Trading panel, removing the chart, hiding
the workspace or not rendering a window stops execution. Second, the chart adapter turns the last
candle close into a quote stamped with the current wall clock and `is_market_open = true`; an old
Friday stock candle can therefore fill an order during the weekend while bypassing the engine's
stale-data and market-hours checks. Third, margin can be bypassed by modifying an accepted order or
by placing several nominally position-reducing orders. Fourth, protective orders survive a
position reversal with their old side, so a supposed stop or take-profit can increase the new
position instead of protecting it.

The main medium-severity issues are an advertised Day duration that never expires, incomplete
iCloud takeover behavior for rejected submissions, swallowed action errors and a non-functional
export command, and large per-frame/per-fill snapshot copies and table rewrites. The persisted
domain also lacks a load-time integrity pass, while several modeled settings and states are unused
or unreachable.

The four high findings should be fixed before paper trading is presented as reliable. Because no
real money is routed, these defects do not create brokerage loss, but they do make the simulated
ledger materially different from the orders the user believes they placed. That undermines the
feature's primary purpose.

Severity meanings used below:

- **High:** a realistic workflow can silently miss, invent or materially mis-size executions or
  positions.
- **Medium:** a user-visible correctness, durability, availability or performance problem with a
  plausible trigger.
- **Low:** bounded polish, auditability, maintainability or hardening work.

## Resolution status

Updated after the fix PRs. Revision reviewed: `8b76f8e`.

| Finding | Status | PR |
|---|---|---|
| High: execution tied to the panel | Partly fixed (no app-owned feed) | #299 |
| High: forged freshness and market-open | Fixed (replay still quotes live) | #299 |
| High: margin bypass | Fixed | #300 |
| High: protective orders after reversal | Fixed | #300 |
| Medium: Day orders never expire | Mitigated (control removed) | #301 |
| Medium: rejection does not take ownership | Fixed | #301 |
| Medium: swallowed errors, Export | Fixed | #301, #295 |
| Medium: whole-snapshot cost | Partly fixed (fill save still full rewrite) | #302 |
| Medium: no integrity pass | Fixed | #302 |
| Low: dead settings and states | Not changed | |
| Low: UTC history, accent colour | Fixed | #303, #295 |

Still open: an app-owned quote feed with defined offline behaviour, Day-order expiry, replay-mode
quotes, an append-only save, and a real-window pass of all of the above. The review's own
end-to-end soak against a reference ledger has not been run.

## Findings

### High — Order execution stops with the presentation that happens to supply its quote

**Status: partly fixed (#299).** Quotes now flow with the Paper Trading panel closed, for instruments that have a working order or position. Still true: orders run only while a chart for the instrument is rendered; nothing fills while the app is quit, asleep or showing no such chart. The app-owned feed is not built and is documented in `docs/agent-guides/market-data.md`.

`ChartWorkspace::sync_paper_quotes` returns immediately unless that workspace's Paper Trading panel
is visible. It then supplies quotes only for market charts currently present in that workspace
(`src/workspace/paper.rs:14-31`). There is no app-owned order quote coordinator and no other
production caller that feeds market quotes into `PaperHost`.

This makes a persisted working order dependent on UI lifetime:

1. Open a BTC chart, open Paper Trading and place a resting limit order.
2. Close the panel, close the chart, switch to a non-rendered workspace, or leave the app without a
   rendered chart for that instrument.
3. Let the market cross the limit.
4. Reopen the panel later. The order is still working because no quote was processed while the
   relevant UI was absent.

The problem is especially surprising because the account is app-wide and durable, while execution
is window-local and transient. Several windows may render charts, but none owns continuous demand
for the symbols with working orders. iCloud's single-owner rule prevents two Macs from filling the
same account; it does not make the owner continue running it when its panel is closed.

Recommendation: move paper quote demand out of `ChartWorkspace` into an app-owned coordinator.
Derive the required instrument set from locally owned working orders and open positions, subscribe
or poll by provider while the app is active, and make panel/chart visibility presentation-only.
Define and display the behavior while the app is quit, asleep or offline; if background execution
is not supported, reconcile from provider bars on resume or explicitly pause orders rather than
silently pretending they remained live. Add lifecycle tests for panel close, chart removal,
workspace switching, hidden windows, sleep/resume and relaunch.

### High — The chart quote adapter forges freshness and market-open state

**Status: fixed (#299).** The quote timestamp is when the candles arrived (`Chart::last_data_ms`); a price older than its source's window is withheld; Alpaca outside US hours is sent with `is_market_open = false`. Not adopted: rejecting equal timestamps (broke same-millisecond engine tests; arrival times make collisions between charts moot). Not fixed: in replay the quote is still the live price.

The adapter takes `chart.candles.last().close`, assigns `SystemTime::now()` as its timestamp, and
sets `is_market_open: true` for every supported source (`src/workspace/paper.rs:20-49`). The engine's
actual protections compare that supplied timestamp with a 30-second maximum age and skip a quote
only when `is_market_open` is false (`src/model/paper_engine.rs:485-500`). Consequently, neither
protection can fire for production chart quotes.

An Alpaca chart last refreshed at Friday's close can be opened on Sunday. Its old close is relabeled
as a fresh Sunday quote and may fill a market, stop or limit order. A slow or failed REST provider
has the same problem: as long as a positive candle remains cached, the adapter refreshes its quote
timestamp every normal chart refresh interval. The fill history records `LastPriceFallback`, but
does not reveal that the underlying candle was stale.

Multiple chart windows make the result nondeterministic. Each chart stamps its own cached close
with the current millisecond; the engine rejects only strictly older timestamps, so equal timestamps
are accepted (`src/model/paper_engine.rs:367-390`). A later-rendered stale chart can overwrite a
fresher chart's quote and evaluate every order again.

Recommendation: carry provider observation time and market-session state into `PaperQuote`. Never
substitute receipt time for market-data time. Centralize quote arbitration by instrument, reject
equal-or-older observations unless the new observation has a stronger source, and define a source
priority such as live bid/ask, live last trade, REST candle close. For Alpaca, use verified session
state; for 24/7 sources, declare that capability explicitly. Tests should cover an old candle,
weekend stock data, a failed refresh with cached candles, future timestamps, equal timestamps and
two windows with different snapshots.

### High — Margin reservations can be bypassed after acceptance

**Status: fixed (#300).** `modify` runs the submit funds check; an older working exit claims its share of the position so overlapping exits reserve for the short they could open; commission counts for orders that add exposure (a pure close is never refused for its fee); a fill rechecks funds at the executed price and is rejected with an event when it no longer fits.

Initial submission computes `reserved_margin` and compares it with available funds. Modification
recomputes the reservation but never performs that comparison (`src/model/paper_engine.rs:309-352`).
A one-unit affordable limit order can therefore be accepted, modified to a hundred units, and
later filled even though the larger order would have been rejected at submission.

There is a second bypass in the reservation model. `required_margin` treats each opposite-side
order as reducing the entire current position without considering other working orders that have
already reserved that reduction (`src/model/paper_engine.rs:958-980`). For a long position of one
unit, any number of one-unit sell orders can each reserve zero. If several trigger, the first closes
the long and the remaining orders open and grow a short position without ever reserving its margin.
The fill path does not perform a final buying-power check, and commissions are not included in the
funds requirement (`src/model/paper_engine.rs:552-603`). A stop gap or a large configured commission
can likewise take the account beyond its admitted risk.

The ticket's funds check is only advisory UI logic (`src/model/paper_ticket.rs:475-480`). The engine
must remain authoritative for modifications, chart drags, synced state and future callers.

Recommendation: implement account-level exposure reservation rather than calculating orders in
isolation. Allocate each unit of reducible position to at most one non-OCO working exit, net mutually
exclusive OCO children as one reservation, and reserve margin for the remaining worst-case
exposure. Re-run the same invariant atomically on submit and modify. At fill time, re-evaluate
buying power using the actual execution price, slippage and commission; define whether an order is
rejected, partially filled or allowed to create a documented deficit. Add tests for oversized
modification, three overlapping exits, opposite orders across OCO groups, gap-through stops,
commission shortfall and concurrent orders on multiple instruments.

### High — Protective orders can increase exposure after a reversal

**Status: fixed (#300).** After every position change, protection that no longer reduces the position is cancelled, protection larger than the position is shrunk (never grown), bracket children are sized to the position the entry left, and every change emits an event (including the sibling cancel on a TP/SL fill). Implemented as a cap by position size rather than a per-bracket allocation model; same guarantee, a protective fill can at worst close the position.

Every fill calls `resize_protective_orders` with only the absolute remaining position size. The
function resizes every working non-entry order for the account/instrument, without checking the
position's new side, parent order or OCO group (`src/model/paper_engine.rs:714-729`, `838-860`).

Example:

1. Buy one unit with a sell take-profit and sell stop-loss.
2. Reverse the position by selling two units.
3. The position is now short one unit, but both old protective orders remain sells and are resized
   to one unit.
4. When either triggers, it adds to the short instead of closing it.

Multiple bracket entries have another failure mode. A manual partial close resizes every bracket
group to the full remaining position, so several independent groups can each close that same
remainder and eventually reverse it. When the position reaches zero the resize path cancels the
children by changing their status directly, without emitting cancellation events, leaving the
order-event audit incomplete.

Recommendation: give protective orders an explicit protected-position or bracket allocation.
Cancel old-side protection on a flip, resize only the allocation belonging to the affected bracket,
and create replacement protection only through an explicit user choice. Route every automatic
cancel/resize through the same transition/event machinery as manual changes. Test full close,
partial close, flip, two bracketed entries, a bracket plus a manual exit, and OCO audit history.

### Medium — Day orders are offered but never expire

**Status: mitigated (#301).** The Time-in-force control is removed from the ticket. Day expiry is not implemented; the enum and stored values remain.

`PaperTimeInForce::Day`, `PaperOrderStatus::Expired` and the corresponding event are persisted and
shown by the UI (`src/model/paper.rs:53-84`, `484-496`). The evaluator never examines
`time_in_force`, never calculates a session deadline and never transitions an order to Expired.
There is no expiration timestamp in `PaperOrder` (`src/model/paper.rs:353-390`). A Day order is
therefore behaviorally identical to Good-Til-Canceled and can fill days later.

Recommendation: either remove/disable Day until it is implemented or persist the relevant trading
session deadline at acceptance. Expire it from an app-owned clock/session driver even when no quote
arrives, emit an Expired event, release its reservation and synchronize the transition. Tests need
Friday/weekend boundaries, exchange timezone and daylight-saving changes, restart before expiry,
offline expiry and an account mirrored on another Mac.

### Medium — A rejected action mutates a mirrored account without taking ownership

**Status: fixed (#301).** The host claims a mirrored account when the candidate snapshot changed, including a recorded rejection, in the same save. A refusal that changed nothing (invalid ticket, unknown order) leaves the owner alone. Covered by a two-Mac test.

The engine deliberately appends a rejected order and audit events before returning its domain
error. `PaperHost::apply_for` persists that candidate, but claims an account owned by another Mac
only when `result.is_ok()` (`src/host/paper_host.rs:433-484`). Thus submitting an invalid or
underfunded order against a mirror changes and queues the account while its owner remains the other
device. `track_paper` bundles the modified rows with that unchanged owner
(`src/store/sync/paper.rs:283-312`).

This contradicts the documented rule that acting on a mirrored account takes it over and creates a
needless concurrent version/conflict risk. It also means this Mac shows its rejected audit locally
while still refusing to fill that account's existing working orders.

Recommendation: distinguish a read-only failure from an operation that intentionally created an
audit mutation. Claim mirrored ownership whenever a user action changes the candidate, including a
recorded rejection, and commit the owner change and paper snapshot in one store transaction. If a
rejected attempt should not count as acting, do not mutate or upload the mirrored bundle. Add a
two-store test for rejected submit, invalid modify and stale cancel, with explicit expectations for
ownership, outbox contents and convergence.

### Medium — Action failures are routinely discarded and bulk operations can hide partial success

**Status: fixed (#301; Export in #295).** Failures go to the panel's dismissible notice, bulk actions report "Closed 1 of 3 positions" with the first reason, the account sheet stays open with a "Not saved" card, and chart-overlay failures reach the panel. Export Trading Data was implemented on main before this review was acted on.

Panel, overlay, account and destructive actions commonly use `let _ = host...`
(`src/host/paper_host.rs:721-841`, `src/workspace/paper.rs:111-128`). Individual host operations set
`last_error`, so some failures appear in the panel on a later frame, but a later successful
operation clears that shared error. `close_all_positions` and `cancel_all_orders` return one result
per item, then the UI discards the whole vector (`src/host/paper_host.rs:366-387`, `775-782`). A
failure followed by a success is therefore invisible, while the account is only partly changed.

Account create/reset sheets close even when persistence failed. A failed delete dismisses its
confirmation. Dragging an order marker gives no local error or snap-back explanation beyond the
generic panel banner. Separately, both visible **Export Trading Data…** commands produce
`PaperPanelAction::Export`, but the workspace deliberately does nothing with it
(`src/ui/paper_header.rs:342-357`, `src/workspace.rs:641-647`).

Recommendation: make action handlers consume and present every result. Bulk actions should produce
one summary containing successes and named failures, retain failed items, and never let a later
success erase an earlier error. Keep sheets open on failed persistence. Either implement an atomic
snapshot/CSV export with a save dialog and error reporting or remove/disable the command with an
honest explanation. Add injected SQLite-failure tests at each UI boundary and a mixed-success bulk
test.

### Medium — Rendering and filling scale with the complete lifetime snapshot

**Status: partly fixed (#302).** Per-frame snapshot cloning is gone (lists rebuild only when `PaperHost::revision()` changes), a tick nothing can fill is recorded in place, and a tick that changes no ledger row is not saved. In release at 5,000 fills a tick with a working order dropped from 80 ms to 6.9 ms. Not done: an account-scoped append-only save; a submit or fill is still a full rewrite (about 84 ms at 5,000 fills).

The panel clones the entire `PaperTradingSnapshot` every frame and then clones the selected
account's positions, orders, events, trades and journal again (`src/host/paper_host.rs:577-627`). It
does this while the app holds the shared `PaperHost` mutex across the workspace UI. The documented
sync capacity reaches thousands of fills and multi-megabyte journals, so this is not a theoretical
small collection.

Every evaluated quote also clones the complete snapshot to make a candidate
(`src/host/paper_host.rs:318-319`). If an order is working, a save deletes and reinserts every row in
all paper collections and serializes the complete quote map (`src/store/paper.rs:38-83`). The same
candidate/save pattern is appropriate for atomicity, but its current granularity makes a price tick
or one fill O(total lifetime history across all accounts). It also rebuilds sync bundles after each
save, relying on hashes to avoid uploads rather than avoiding the work.

Recommendation: first remove frame-time whole-snapshot clones by borrowing immutable slices or
building a lightweight selected-account projection cached by a revision counter. Then benchmark
fill latency and UI frame time at 100, 1,000 and 5,000 fills. If needed, split transient quotes from
the persisted snapshot and add an account-scoped store transaction that appends immutable fills,
events, trades and journal entries while updating only affected account/order/position rows. Keep a
single atomic boundary and the one-record sync projection. Set explicit frame, fill and database
size budgets before choosing the storage refactor.

### Medium — Deserialization checks shape, not paper-ledger integrity

**Status: fixed (#302).** `PaperTradingSnapshot::integrity_issues()` checks duplicate ids, missing account/order/parent references, duplicate positions, overfilled orders and non-finite numbers; it runs after load (refuses to open), after reload (keeps the last good state) and before every save. Not checked on purpose: that the cash balance equals the ledger, because an older or imported account may differ and refusing it would lock the user out.

Store reads are commendably strict about JSON decoding, but a successfully decoded snapshot is
passed directly to `PaperTradingEngine::new`. There is no integrity check for duplicate ids,
missing accounts, orders referencing the wrong account, invalid OCO/parent links, duplicate
positions, non-positive instrument increments, non-finite monetary values, filled quantity above
original quantity, or account balances inconsistent with the ledger. Public engine methods also
validate request fields selectively; account settings are accepted without a domain validator.

This matters because paper bundles cross versions and Macs as JSON, and because a single malformed
numeric or relationship can affect all future margin and fill calculations. The generic sync
validator cannot prove these domain invariants.

Recommendation: add `PaperTradingSnapshot::validate()` and run it after SQLite load, after inbound
bundle assembly and before every save. Fail closed without replacing the last good in-memory state,
surface the offending account/record, and provide a recoverable export or quarantine path. Reuse
small validated value types or centralized checks for finite positive money, leverage, commission,
slippage, tick size and quantity increments. Add property tests over generated snapshots plus
targeted corrupt-row tests for every relationship.

### Low — Several modeled controls and transitions are incomplete or misleading

**Status: not changed.** `show_executions_on_chart`, `instant_order_placement`, `MarketClosed`, `PendingCancel` and partial fills come from the ported Swift model and are synced; removing them is a schema decision. `MarketClosed`'s copy ("the paper order remains pending") matches what a market order placed while closed does, silently.

`show_executions_on_chart` and `instant_order_placement` are persisted settings but have no runtime
consumer (`src/model/paper.rs:185-215`). Chart configuration reads only the position and order
toggles (`src/workspace/paper.rs:73-95`). Partial fills are modeled, but the only execution path
always fills `remaining_quantity()` in one operation. `MarketClosed` has user-facing copy, but the
evaluator silently returns `Ok(())` for a closed quote instead of producing that error. Pending
cancel exists only as an in-memory intermediate immediately replaced by Canceled.

These are not individually dangerous, but they enlarge the persisted surface and give future code
false confidence that behavior exists. They also make compatibility promises for states that have
never been exercised.

Recommendation: classify each field/state as supported, planned or legacy. Implement and test the
supported behavior; otherwise remove it before a stable schema depends on it. If partial fills are
deliberately out of scope, simplify the model and document unlimited simulated liquidity. If they
remain, define liquidity, commission and reservation behavior before exposing them.

### Low — History uses UTC and the paper UI extends the known accent-colour violation

**Status: fixed (#303; accent in #295).** History tables use the local UTC offset (one offset per frame, so rows across a daylight-saving change are an hour off). The account-sheet accent colour was already removed on main.

The panel passes a fixed `utc_offset_secs: 0` to every history table
(`src/host/paper_host.rs:658-687`) even though the application already has a local timezone service.
Users outside UTC see order, trade and journal times shifted from local time and from the Portfolio
tab.

The account sheet uses the blue accent for its header, leverage selection and create button
(`src/ui/paper_account_sheet.rs:275-292`, `376-390`). The repository guidance explicitly calls the
paper account submit button a known breach and says not to extend it. The order ticket's side colour
is meaningful trading data, but the generic account creation action should use the neutral sheet
primary style.

Recommendation: pass the timestamp-specific local offset used elsewhere in the app and test a
non-UTC zone plus a daylight-saving boundary. Replace non-semantic account-sheet accent treatments
with the neutral control styles prescribed by `docs/agent-guides/core.md`; retain green/red/orange
only where they communicate trading state or destructive risk.

## Validation performed

`cargo test paper --lib`:

- 131 passed
- 0 failed
- 1 ignored
- 5,246 filtered out

This filter covers the paper model, engine, ticket, store, host, sync ownership/conflict behavior,
panel components, chart overlays and relevant workspace/context tests. It does not exercise the
corpus suite or a live market session.

`cargo clippy --all-targets -- -D warnings` completed successfully.

No manual GUI pass, live Binance/Alpaca/prediction-market session, weekend/session-boundary run,
sleep/wake test, network-drop test, multi-window quote race, large-history frame profile or injected
disk failure was performed. The high findings above follow directly from the production call graph
and state transitions, but their eventual fixes need those integration scenarios.

## Recommended production-readiness order

1. Replace UI-scoped quote delivery with an app-owned, provider-aware paper execution feed and
   honest offline/session semantics.
2. Enforce account-level worst-case margin on submit, modify and fill.
3. Redesign bracket allocation so close, partial close and reversal cannot turn protection into
   exposure.
4. Implement or remove Day orders and non-functional export before release.
5. Make ownership changes and rejected audit mutations transactional and convergent across Macs.
6. Stop swallowing action results; add persistence-fault and partial-bulk feedback tests.
7. Add snapshot integrity validation, then profile and remove whole-snapshot work from frame-time
   and common fill paths.
8. Reconcile dead settings/states, localize history timestamps and bring the paper sheets into the
   repository's neutral-control style.

After steps 1-4, run an end-to-end soak with several simultaneous stock and crypto orders across
market close, network interruption, window closure, sleep and relaunch. Compare every order event,
fill, position, margin figure and closed trade against a deterministic reference ledger before
calling the feature production-ready.
