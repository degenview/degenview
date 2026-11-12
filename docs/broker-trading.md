# Live trading (brokers)

DegenView can trade for real through a broker, next to paper trading. All of the exchange-facing
work lives in a separate crate, [`degenview-brokers`](https://github.com/degenview/brokers),
pinned in `Cargo.toml` to one exact crates.io version. This page is about how the application uses
it. What was verified against each exchange, and what was not, is in the crate's own `docs/` (for
Coinbase: `docs/coinbase.md`).

Read this with [Market data](agent-guides/market-data.md) (the data a chart shows is not the
price an order trades at) and [iCloud Sync](icloud-sync.md) (nothing here syncs).

## The rules

* **Real money.** Correctness and secrecy come before convenience. Live trading is visibly
  different from paper trading everywhere it appears.
* **The exchange is the source of truth.** Balances, orders and fills are what the broker last
  said. Nothing in the app invents or removes one: a cancel is a *request*, an order leaves the
  working set only when the broker reports it finished.
* **No broker is named in the app's broker layers.** `host/broker_host.rs`,
  `service/keychain_broker_secrets.rs` and `ui/settings/brokers.rs` draw everything from the
  crate's registry and each broker's descriptor; a test fails if one names an exchange. Adding a
  broker is a change to the crate (see its `docs/adding-a-broker.md`), plus enabling its Cargo
  feature here.
* **No side doors.** Pine scripts, alerts, webhooks and MCP have no path to a broker: a test fails
  if anything under `mcp/`, `pine/`, `pine_studio/` or the alert and webhook hosts mentions the crate
  or the host. A future automated-trading feature needs its own design and its own permission.
* **No funds movement.** The app asks for no permission that moves funds, and warns when a key has
  it.

## Where things live

| Piece | Where |
| --- | --- |
| Credentials | Operating system secret store, app-only group `es.nico.degenview.broker`: `service/keychain_broker_secrets.rs` (the crate's `SecretStore`). Read when connecting, never at startup for a user who has none. **Every read, write and removal runs on one secrets thread owned by `BrokerHost`** (a queue, so a save followed by a removal cannot swap), never on the interface thread: the OS store may prompt or hang. `connection()` reads only what that thread last learned (a key shows as saved once the store has answered). A removal waits behind an unanswered Keychain prompt on the same queue; the connection itself stops at once. Never SQLite, never iCloud, never a log. |
| Unconfirmed orders | Local-only setting `broker.pending` (`host/broker_host.rs::StoreJournal`): written **before** an order is sent, so a restart cannot lose an order that may be live. An unreadable record is an error, never "nothing pending". Every record carries the **account** (the portfolio) that sent it and is keyed by `(broker, account, client order id)`; only a session on that account shows, looks up, settles, retries or deletes it. Records of another account, and records from before they carried one, are left untouched and counted ("1 unconfirmed order belongs to a different account"); they are checked when that account connects again. |
| Choices about trading | Local-only setting `broker.prefs`. |
| Connections | `host/broker_host.rs::BrokerHost`: one per process, shared by every window, **outside the paper-trading lock**. Each connection's runtime runs on its own thread and publishes `Arc` snapshots; the UI never calls the network. Every request for a connection (a new key, the saved key at startup, Disconnect) takes a number, and a start may publish its runtime only if its number is still the broker's current one, so a Disconnect, or a newer key, always has the last word over a start that was still waiting on the Keychain prompt or the store. |
| Which venues are on | `host/trading_hub.rs::TradingHub` is the **only owner of a venue's lifecycle**: `enable`, `connect(id, credentials)` (save the key, connect, remember the venue as enabled) and `disable` (forget the choice, stop the connection, remove the key). Settings calls these, not `BrokerHost`: disconnecting only the host left a key-less broker (the sandbox) in `trading.enabled`, so it reconnected at the next start. `connect` and `disable` return a polled `Operation` (the answer of the secret store), shown as "Saving the key…" / "Removing the key…". |
| Redraws | `BrokerHost::revision()` is compared once a frame in `DegenViewApp::tick_tab_quotes`, which repaints every tab window. |
| Settings | Settings ▸ Brokers (`ui/settings/brokers.rs`): connect (the key is checked with reads only, then saved), permissions, Disconnect. |

## Connecting

Settings ▸ Brokers shows a card per broker. The broker's own setup steps say how to create a key
(and to leave Transfer off). **Connect** checks the key by connecting with reads only: nothing is
placed or changed. If it works, the key is saved to the secret store, the connection starts, and the
fields are emptied; a pasted private key is never left on screen (the field shows only its length).
A key that can move funds gets a warning; a key that can only view says so, and orders are not
offered. **Disconnect** removes the key from this Mac and stops the connection. It does not revoke
the key at the broker, and orders already placed stay live there; the dialog says both. If the
operating system refuses to remove the key, Disconnect still stops the connection but says plainly
that the key is **still stored** and will connect again when the app starts (it is never reported as
removed). Removing a key that is not stored counts as removed.

## The panel

The Paper Trading panel is the trading panel: when a broker is connected, a "Trade on" switch
(Paper | Coinbase...) appears above it, per tab (`ChartWorkspace::trading_venue`, never saved: a tab
starts on paper). On a live venue the panel is `ui/broker/panel.rs`, with an amber rule and a
`LIVE · NAME` badge (`ui/broker/badge.rs`; amber is not the buy/sell colour and never blue) that
paper trading never uses. The paper account is only created while the venue is Paper, and paper
overlays on charts are hidden while it is live.

It draws what the venue last said (`TradingBackend`) and nothing else, so every window shows the same state.

* **Orders**: working orders and the ones that just finished (dimmed), plus *unconfirmed* sends. Pick
  rows to cancel several at once. A cancel marks the row "Cancel requested" and leaves it listed until
  the broker confirms; an order that filled during the cancel says so.
* **History** (finished orders, filterable, paged), **Fills** (the snapshot's recent fills, older ones
  on request) and **Balances** (available and held, never a dollar value that was not read from the
  broker).
* A banner says when the connection is down (what is shown may be out of date and new orders are
  paused), when the key was revoked, and when it can view but not trade.

## Ordering

**New order** opens the ticket for the focused chart's market, if the chart's data source is the
broker's: `Broker::resolve_market` maps it, and a chart whose data comes from anywhere else is
refused with a sentence, never traded on a similar market. The order sheet (`ui/trade/order_sheet.rs`, rules in
`model/broker/ticket.rs`) takes the market's rules and the best bid and ask from the broker, not the
chart, and says so. It offers only what the market allows. Text is parsed exactly (`0.001` stays
`0.001`); a value off the market's increment is reported with the nearest valid value and **never
rounded**. The broker's own preview (fees, total, warnings, refusals) replaces the estimate when it
arrives.

**The broker's preview is required.** Review is enabled only on a *fresh, accepting* preview of the
order exactly as it stands (`model/broker/ticket::preview_gate`): no answer yet, a failed preview, a
preview that lists problems, or one older than a minute all leave it off. A failed preview says so
and offers "Try again". The broker's create endpoint does not require a preview, so this is this
application's rule, not Coinbase's. Orders are also paused while the record of unconfirmed orders
cannot be read (below).

**An order goes to the account it was reviewed against.** The sheet reads the account and the
connection once (`TradingBackend::binding`: the portfolio id and the runtime's generation) and uses it for
the preview, the review and the send. If either changes while the dialog is open (another key, another
portfolio) the review closes, "The account changed... Nothing was sent", and the venue refuses the old
review should anything still send it; the crate's runtime checks the account again on its own thread
before recording or sending anything. The account is shown with the last characters of its id, so two
"Default portfolio" accounts read differently. A review whose preview has aged past a minute while
the dialog was open is dropped too.

Review opens a confirmation with the account, market, side, type, size, price and **the broker's
estimated total, fee and average price and any note it attached**, taken from the preview of exactly
this order (not from the best prices the app last saw). A **market order, an edit and Cancel all always ask**; confirming a limit or stop-limit order, or a
single cancel, is a setting (Settings ▸ Brokers; the defaults ask for orders and do not ask for a
single cancel). The dialog starts with no button focused, so a stray Enter sends nothing. The order
is then handed to the runtime, which records it in the pending journal before it sends it, sends it
once, and shows it as *Sending* / *Unconfirmed* until the broker lists it.

## Editing and cancelling

The pencil edits an open limit or stop-limit order's price and size **in place** (the broker's edit).
"Review change" follows the same rule as the ticket: only a fresh, accepting `edit_preview` of the
exact change enables it, and the confirmation shows the broker's total and fee. The sheet always
shows the order as it is **now** (a fill or an edit made elsewhere while it is open updates it), the
preview is asked for again when the order changes, and the amend carries the fingerprint of the order
that was reviewed: if the order changed in between, nothing is sent and the order is read again. The pencil is
disabled, with the reason, for anything else. Cancel-and-replace is not
offered anywhere. The confirmation says the order can still fill at its old terms until the change
is applied and that a new price may lose its place in the queue.

## When the record of unconfirmed orders cannot be read

Orders sent and not yet confirmed are kept in the local `broker.pending` setting. If it cannot be
read (damaged data, a locked store), orders that may be live could be missing from every list, so the
runtime publishes `journal_error`, the panel shows it, and **no new order is accepted** (the ticket,
the chart's buttons and the runtime all refuse, each with the reason). Reading, cancelling and editing
keep working, so a user can always get out. The runtime tries to read the record again on every
reconcile round and lifts the block by itself. An order the broker accepted stays on screen and on
record even if the record cannot be closed, and is closed on a later round. A cancel whose answer was
lost shows as "not confirmed" and the order is read again.

## Notifications

Orders placed, partly filled, filled, cancelled, expired or rejected produce a message at the foot of
the focused tab and a system notification, once each (the crate's book deduplicates replays, and the
orders already open when a connection starts are not announced). Settings ▸ Brokers turns them off
(`broker.prefs`, local to this Mac).

## Trading from a chart

With a broker as the tab's venue, a chart whose data comes from **that broker's own source** (the
descriptor's `market_source`: a Coinbase chart for Coinbase) gets live controls; every other chart is
left alone, and the menu says why instead of trading a similar market. The rules are pure
(`model/broker/chart.rs`); `workspace/broker.rs` fills each card each frame.

* **Header SELL / BUY** show the broker's own best bid and ask (read for as long as a card shows them,
  every two seconds, never for a chart nobody is looking at: `BrokerHost::want_quote`), never the
  chart's last price, which is another feed's. A price is shown only if it is fresh both here (ten
  seconds since it arrived) and by the exchange's own time on it: a price that sat on the way is not
  stamped as new (a time in the future, from a clock that is behind, is not hidden; a price with no
  time at all is an error). A clock more than ten seconds ahead of the exchange's hides prices. They open the ticket; nothing is placed from them.
  Greyed, with the reason as the tooltip, when orders are not possible (reconnecting, view-only key,
  market not tradable, replay).
* **Order lines**: one per working order, on the chart's price scale, tagged `LIVE`. A line is the
  order *as the broker has it*. ✕ asks to cancel (the single-cancel setting decides whether it asks
  first) and the line stays, marked "cancel requested", until the broker confirms. Clicking the pill
  opens the edit sheet. **Dragging** shows where the line would land (on the market's price step) and,
  on release, opens the edit sheet with that price: the broker is asked nothing until the user has
  seen the preview and approved the change. Escape drops the proposal; the line only moves when the
  broker's order changes. A line that cannot be edited (not open, a stop-limit the broker will not
  edit, a cancel in progress) cannot be dragged.
* **Right-click** adds "Live · Buy / Sell … at market / limit @ price / stop-limit @ price" entries in
  place of the paper ones. They open the ticket filled in from the click (the price is put on the
  market's price step, exactly), never place an order. Greyed with the reason when it could not be
  placed from this chart, and the price entries are greyed on a Renko, Kagi, Heikin Ashi... chart,
  whose prices are constructed, not traded.
* **Replay** refuses all of it: the pills and menu entries are greyed, a ticket that is open when a
  replay starts is closed unsent, and a line cannot start an edit. Cancelling stays possible.
* The ticket, the edit sheet and the cancel question are drawn by the workspace even when the trading
  panel is closed, so starting an order from a chart does not need the panel.
* Live orders are **never** drawn into a shared snapshot of the chart.
* Every window showing the market draws the same lines from the same snapshot; an order placed in
  one appears in all of them, once, and a cancel updates every one.

## Trying it without an account

The broker crate ships a mock exchange (the stateful fake its own tests use, built from the
exchange's published specification). To drive the whole flow in a real window with no account and no
money:

```bash
cd ../degenview-brokers && cargo run --example mock_exchange --features testing
```

It prints a throwaway key and the two addresses. Then start the app with those and the
`dev-mock-broker` feature (never enable it for a distributed build: it compiles the mock in and reads
the variables; the crate honours loopback addresses only, so a stray variable cannot redirect a signed
request):

```bash
DEGENVIEW_MOCK_BROKER_URL=http://127.0.0.1:<port> DEGENVIEW_MOCK_BROKER_WS_URL=ws://127.0.0.1:<port> \
  DEGENVIEW_NO_KEYCHAIN=1 cargo run --release --features dev-mock-broker
```

and paste the printed key in Settings ▸ Brokers. Place, edit and cancel orders from the panel and the
chart; the mock keeps its own state, so nothing leaves the machine.

## Verification

Automated (all against the mock; none can place a real order): the crate's tests (including its
conformance suite and the two-account tests), and the app's tests of the host, the screens (confirm-then-send, edit, cancel,
drag, Escape, the ✕), the workspace and the menu. Pictures for review are written by
`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_live_trading -- --ignored` (and
`gallery_of_chart_trading`). Not done: the screens have not been driven in a real macOS window, and
nothing has run against a real Coinbase account. Both are needed before this ships; see the pull
request descriptions for the checklist.

## Not done yet / not verified

* **The venue layer is the path for the panel and the chart.** What the screens *ask* a broker goes
  through `BrokerCommands` (`trading/broker_commands.rs`: retry, cancel, paged history and fills, and
  the edit as `trading::edit::EditSession`); what they *read* goes through `TradingBackend`; a test
  fails if `ui/` or `workspace/` name a broker's runtime or the host (Settings ▸ Brokers excepted).
  Still on `BrokerHost`: Settings ▸ Brokers (it manages connections). The user's choices are
  `TradingHub::prefs`. The hub is handed to each window (`WindowEnv`), not read from a global.
  `trading/` does not import `ui` (a test checks). The findings, evidence and proposed fixes are in
  [reviews/broker-architecture-follow-ups.md](reviews/broker-architecture-follow-ups.md).

* Two Coinbase portfolios have not been tried together against the real service. The account id is
  the key's `portfolio_uuid`; a key without one is the account `default`, which two such keys would
  share. The two-account behaviour is tested against two fake exchanges sharing one journal.
* A key the operating system would not let go of (a failed Keychain, Credential Manager or Secret
  Service removal) is reported, not worked around; the real failures could not be provoked here.
* The AI-key, snapshot-secret and webhook stores still remove their secrets best-effort.
* Nothing here has been run against a real account by the author of the change. The mock
  exchange is built from Coinbase's published specification; see the crate's `docs/coinbase.md` for
  what could not be verified.
* The ticket offers good-til-cancelled and fill-or-kill, not good-til-date (it needs a date picker).
  Brackets, TWAP and scaled orders are not implemented.
* A chart's lines show the working orders of *its* market only; a position-style summary does not
  exist (spot balances are not positions).
* Figures use `.` for decimals and `,` for thousands whatever the system locale; typed numbers accept
  `.` or `,` as the decimal separator but never a thousands separator.
* The shared widgets of the paper ticket were not extracted: the live ticket builds from `kit`
  controls, so paper trading is untouched. The one shared change is the pill's hit testing in
  `paper_chart_trading::marker`, which now also reads a click on the ✕ that lands on the pill's body.
