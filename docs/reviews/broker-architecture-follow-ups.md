# Live trading: architecture review, what is still open

An external review of PR #457 (head `5fc8e67`) found the venue layer half migrated: pure helpers, account
binding, preview gating and the conformance tests are sound, but broker-specific workflows run in parallel
with the venue-neutral ones. Each claim was re-read in the code. This page lists what **was not fixed in
#457** and how to fix it. What was fixed (commit `9206dc7`) is at the end, so a reader can tell the two apart.

Read [Live trading](../broker-trading.md) first. Line numbers are those of `9206dc7`.

## Open findings

### 1. The edit-order state machine lives in a render function (P1)

**Evidence.** `edit_sheet` is `ui/broker_panel.rs:1145` and runs about 470 lines. One function holds
reconciliation of the live order (`edit.order` refreshed each frame), validation and market increments, the
order fingerprint, preview debouncing and freshness, polling of the async preview, invalidation of a review
when the order or the account changes, dispatch of `amend`, confirmation, and drawing. The money-moving
rules (do not amend an order that changed since it was previewed) are interleaved with layout code, so a
UI tweak can change them, and the transitions cannot be tested without a frame loop.

**Proposed fix.** Extract an `EditOrderController` (host side, or `trading/edit.rs`: pure, no `egui`):

* State: `Editing { fields } -> Previewing { key } -> Reviewing { preview, base } -> Sending -> Done | Failed`.
* Typed events in: `FieldsChanged`, `LiveOrderUpdated(Order)`, `PreviewArrived(Result)`, `Confirm`, `Cancel`, `Tick(now)`.
* A read-only `EditView` out (fields, errors, what is enabled, the "old → new" lines) and typed actions out
  (`RequestPreview`, `Amend { change, base }`, `Close`). The panel draws the view and returns the events.
* Every rule that exists today (fingerprint compared against the live order, preview age against
  `PREVIEW_FRESH`, partial-fill check, drop to `Editing` on a changed account) becomes a transition with a
  table-driven test. The existing `broker_panel` gallery tests stay as the drawing check.
* Keep behaviour identical first (move, do not redesign); change nothing in the crate.

**Size / order.** One PR, about 600 lines moved. Do it before the panel migration (item 2): the migration is
easier once the stateful part is not drawn inline.

### 2. The venue layer is not the only path (P2)

**Evidence.** `TradingBackend` serves the order sheet only. The live orders panel calls
`BrokerHost::runtime(..)` from six render sites (`ui/broker_panel.rs:712, 794, 836, 932, 1015, 1152`) for
retry, cancel, paged history and fills, edit preview and amend. Chart order lines
(`workspace/broker.rs`) read broker snapshots through `BrokerHost`. Settings reads `BrokerHost` for cards and
preferences. Edit, retry and paged history/fills are not on the trait, so each venue-specific feature is
written twice (a vocabulary in `trading::model` and another in the panel).

**Proposed fix.** Decide the trait's boundary and write it down, then move screens across in this order:

1. **Cancel and retry** onto the trait (`cancel` exists; add `retry(order_id)`), then the panel and chart
   pill call the trait.
2. **Orders / history / fills reads** as venue-neutral rows (`OrderRow`, `FillRow` come back when a screen
   reads them; they were deleted from #457 because nothing did) behind one `rows(tab, page)` read.
3. **Edit** as `preview_edit` / `amend` once the controller (item 1) exists; paper gets `Unsupported`.
4. Chart order lines read `orders()` from the backend, not the snapshot.
5. Delete `BrokerHost::runtime` from `ui/` (make it `pub(in crate::host)`); a guard test fails if `ui/`
   names `BrokerRuntime` again.

One PR per step; each deletes the broker-specific copy it replaces. A venue added after step 3 needs an
adapter and no screen change.

### 3. Process-wide service locators (P2)

**Evidence.** `BrokerHost::global()` and `TradingHub::global()` are `OnceLock` singletons, read at
`workspace.rs:804,817`, `workspace/broker.rs:33,292`, `workspace/trade_sheet.rs:45,129,196` and
`ui/settings_window.rs:545`. Dependencies are hidden, tests use "first global wins"
(`set_global_for_tests`), two app instances cannot be isolated, and a workspace behaves according to
initialisation elsewhere. The lifecycle bug fixed in #457 was a symptom: two owners reachable by global.

**Proposed fix.** `DegenViewApp` owns `Arc<TradingHub>` and passes it to each workspace through the frame
environment it already builds (the same way the store reaches them). Remove `GLOBAL`, `global()` and the
test hooks; tests build a hub and hand it in. `BrokerHost` stops being reachable except through the hub
(`hub.brokers()`), which also lets `BrokerHost`'s public surface shrink to what the hub and panel need.
Do this after item 2 so the panel does not need `BrokerHost` at all.

### 4. Remaining size and signature smells (P2)

* `order_sheet::form` (`ui/trade/order_sheet.rs:473`) and `number` (`:750`) suppress `clippy::too_many_arguments`.
  Group the arguments into a `FormContext` (market, binding, quote, check) and an input struct for `number`.
* Hotspots after #457: `ui/broker_panel.rs` 1811 lines, `ui/trade/order_sheet.rs` 1382, `trading/broker_backend.rs` 780.
  Items 1 and 2 shrink the panel; split `order_sheet.rs` into `form`, `review` and `state` modules when it is next touched.
* `configure_broker_cards` (Settings) mixes broker projection with mutation; split into a pure
  `cards(hub) -> Vec<CardView>` and an apply step, after item 3.
* `VenueInfo` strings and row projections are rebuilt every frame and ignore a revision. Add a cheap
  `revision()` back to the trait **when a screen reads it** and cache projections on it (it was removed
  from #457 because nothing used it).

### 5. Known limits of what #457 did fix

* **A removal waits behind an unanswered Keychain prompt.** All secret I/O is one queue (so a save and a
  removal keep their order); a delete asked after a stuck read waits. The connection stops at once; only
  the report of the removal is late. If this matters, use one queue per broker.
* **Secret removal elsewhere is still best effort.** AI-key, snapshot-token and webhook stores ignore a
  failed removal. Give them the same `Result` and message as brokers.
* **Cold start.** `connection()` answers "not configured" until the secrets thread has answered, for at
  most the first frame or two after start. The trading hub's "saved key counts as enabled" rule reads it, so
  the venue list can flicker once at startup. Persist `trading.enabled` entries for keyed brokers when a key
  is saved (the hub already does this for `connect`) and drop the rule.
* **Quotes.** A local clock more than 10 s ahead of the exchange hides quotes; a key with no portfolio id
  shares the account `default`.
* **Not verified.** Nothing ran against a real broker account, two real portfolios, or a real failing
  Keychain / Credential Manager / Secret Service, and no screen was driven in a real window.

## Suggested order

1. Edit-order controller (item 1).
2. Panel and chart onto the venue layer, step by step (item 2).
3. Inject the hub, delete the globals (item 3).
4. Argument structs, module splits, projection caching (item 4).
5. Item 5 as needed.

Each step keeps the safety rules intact: an order is previewed and sent against one account and connection
(`Binding`), an amend is checked against the order's fingerprint, an unresolved order is journalled per
account, and nothing here reaches scripts, alerts or MCP.

## Fixed in #457 (for contrast)

* A venue's lifecycle has one owner. Settings calls `TradingHub::connect` and `disable`; turning the
  sandbox off is remembered across a restart (test:
  `disabling_a_broker_that_needs_no_key_is_remembered_across_a_restart`).
* Secret-store reads, writes and removals run on one queued thread; `adopt` and `disconnect` return polled
  results; `connection()` never asks the store (tests: `saving_removing_and_asking_never_wait_on_the_secret_store`,
  `a_key_saved_and_then_removed_while_the_store_was_busy_ends_up_removed`).
* `trading/` does not import `ui` (a guard test checks); `TabKind` and the paper strip text moved out of the
  wrong layer; the unused `TradingBackend` methods, row types and both `allow(dead_code)` lines are gone;
  the hub builds each backend once.
