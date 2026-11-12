# Live trading: architecture review, what is still open

An external review of PR #457 (head `5fc8e67`) found the venue layer half migrated: pure helpers, account
binding, preview gating and the conformance tests are sound, but broker-specific workflows run in parallel
with the venue-neutral ones. Each claim was re-read in the code. This page lists what **was not fixed in
#457** and how to fix it. What was fixed (commit `9206dc7`) is at the end, so a reader can tell the two apart.

Read [Live trading](../broker-trading.md) first. Line numbers are those of `9206dc7`.

## Open findings

### 1. The edit-order state machine lives in a render function (P1): fixed

Done: `trading::edit::EditSession` (pure, time as a parameter, venue answers as polled tasks behind
`EditVenue`) holds every rule; the panel draws its `EditView` and returns `EditEvent`s. Each rule is a
transition with a test in `trading/edit/tests.rs`. Left as it was (a decision, not an oversight): the
sheet closes when the account goes away even while an amend is in flight, and the amend is checked by
the runtime on the order fingerprint (account included) but not on the connection generation.

### 2. The venue layer is not the only path (P2): fixed for the panel and the chart

Done: the orders panel and the chart's order lines draw from the venue only. What the screens *ask* a
broker goes through `trading::broker_commands::BrokerCommands` (retry, cancel, paged history and
fills, and the edit as `EditVenue`); what they *read* goes through `TradingBackend`: `orders()` for
the Orders tab, the header's working count and the cancel question, `notes()` for the banners,
`can_order()`, `recent_fills()`, `balances()`, `chart_unavailable()` and `chart_lines()` for the chart,
and `quote()` for the pills. History and fills pages arrive as neutral `OrderRow`/`FillRow` pages. A
guard test (`the_screens_never_hold_a_brokers_runtime`) fails if `ui/` or `workspace/` name
`BrokerRuntime` or `host.runtime(`, or if drawing code under `ui/` (all but Settings ▸ Brokers) names
`BrokerHost` or `BrokerSnapshot`.

Changed on the way: the panel's account chip now reads like the order sheet's (the label with the last
characters of the account id), and a chart's order lines are drawn from `chart_lines()` whether or not
orders are possible at that moment (as before).

**Still open.** Settings ▸ Brokers reads `BrokerHost` for connection cards and preferences (it manages
connections, so it legitimately holds the host); the workspace asks the host for the user's
preferences (`show_on_charts`, `confirm_single_cancel`, `spend_in_quote_by_default`); the edit
sheet's world (`EditVenue::world`) is still in the crate's types (`LiveOrder`, `ProductSpec`), because
editing in place is a broker-only feature. Preferences could move to `hub.prefs(venue)`; a `VenueInfo`
revision could cache the per-frame projections.

### 3. Process-wide service locators (P2): fixed

Done: `BrokerHost::global/install` and `TradingHub::global/install` and their test hooks are deleted.
`DegenViewApp` owns `Arc<TradingHub>`, gives it to each window through `WindowEnv`, each workspace
takes it with `set_trading_hub`, and Settings reads it from `SettingsInput`. Tests build a hub and hand
it in. The workspace and Settings still reach `BrokerHost` through `hub.brokers()` until the reads in
item 2 move.

### 4. Remaining size and signature smells (P2)

* `order_sheet::form` (`ui/trade/order_sheet.rs:473`) and `number` (`:750`) suppress `clippy::too_many_arguments`.
  Group the arguments into a `FormContext` (market, binding, quote, check) and an input struct for `number`.
* Hotspots after #457: `ui/broker_panel.rs` 1811 lines, `ui/trade/order_sheet.rs` 1382, `trading/broker_backend.rs` 780.
  Items 1 and 2 shrink the panel; split `order_sheet.rs` into `form`, `review` and `state` modules when it is next touched.
* `configure_broker_cards` (`workspace/broker.rs`, not Settings) mixes broker projection with mutation;
  see item 2.
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

1. Crate moves (below), one release.
2. Preferences through the hub, and a revision for the per-frame projections (item 2, open part).
3. Item 5 as needed.

Each step keeps the safety rules intact: an order is previewed and sent against one account and connection
(`Binding`), an amend is checked against the order's fingerprint, an unresolved order is journalled per
account, and nothing here reaches scripts, alerts or MCP.

## What would move to the `degenview-brokers` crate

Pure and egui-free now, so each is a file lift followed by a crate release (0.4.0 if an existing type
changes, 0.3.x if only modules are added) and a version bump here. Nothing was moved: the app pins an
exact crates.io version, and a half-moved rule would be written twice.

| Move | From | Why it belongs there |
| --- | --- | --- |
| Quote cache and freshness (local age plus the exchange's own time) | `host/broker_host.rs` (`want_quote`, `quote`, `tick_quotes`) | A "never show a stale price as fresh" rule; testable with the crate's `ManualClock`, on one clock |
| Edit rules: `EditSession`, `issues`, `edit_blocker`, `PreviewGate` and `PREVIEW_FRESH` | `trading/edit/`, `model/broker/ticket.rs` | The crate already owns `OrderFingerprint`, `OrderChange`, `EditPreview` and the runtime's refusal of a changed order; the rules could be tested against the mock exchange. Needs the crate's `Pending` where the app uses `Task` |
| Ticket text to order, issues, estimate | `model/broker/ticket.rs` | About 70% of it is `validate_draft` already; what is left imports only the crate |
| Account valuation | `trading/valuation.rs` | `Decimal` only |
| Pending-order journal over a two-method storage trait | `host/broker_host.rs::StoreJournal` | `MemoryJournal` repeats the same key and "unreadable is an error" logic |
| Chart access ladder and nearest-increment | `model/broker/chart.rs` | Snapshot and market checks as `BrokerSnapshot::order_access`; replay and the sentences stay here |

Stays in the app: `TradingBackend` and its adapters (the paper account is the second implementation and
they speak UI vocabulary), `TradingHub`, `BrokerPrefs`, the Keychain store, the confirmation lines.
Later and larger: the connection manager and its secrets queue.

## Fixed since

* The edit-order state machine, the commands and reads path for the panel and chart, and the injected
  hub (items 1, 2 and 3 above), and the order sheet's argument structs and split (item 4).

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
