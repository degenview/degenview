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
  `service/keychain_broker_secrets.rs` and `ui/settings_brokers.rs` draw everything from the
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
| Credentials | Operating system secret store, app-only group `es.nico.degenview.broker`: `service/keychain_broker_secrets.rs` (the crate's `SecretStore`). Read when connecting, never at startup for a user who has none. Never SQLite, never iCloud, never a log. |
| Unconfirmed orders | Local-only setting `broker.pending` (`host/broker_host.rs::StoreJournal`): written **before** an order is sent, so a restart cannot lose an order that may be live. An unreadable record is an error, never "nothing pending". |
| Choices about trading | Local-only setting `broker.prefs`. |
| Connections | `host/broker_host.rs::BrokerHost`: one per process, shared by every window, **outside the paper-trading lock**. Each connection's runtime runs on its own thread and publishes `Arc` snapshots; the UI never calls the network. |
| Redraws | `BrokerHost::revision()` is compared once a frame in `DegenViewApp::tick_tab_quotes`, which repaints every tab window. |
| Settings | Settings ▸ Brokers (`ui/settings_brokers.rs`): connect (the key is checked with reads only, then saved), permissions, Disconnect. |

## Connecting

Settings ▸ Brokers shows a card per broker. The broker's own setup steps say how to create a key
(and to leave Transfer off). **Connect** checks the key by connecting with reads only: nothing is
placed or changed. If it works, the key is saved to the secret store, the connection starts, and the
fields are emptied; a pasted private key is never left on screen (the field shows only its length).
A key that can move funds gets a warning; a key that can only view says so, and orders are not
offered. **Disconnect** removes the key from this Mac and stops the connection. It does not revoke
the key at the broker, and orders already placed stay live there; the dialog says both.

## The panel

The Paper Trading panel is the trading panel: when a broker is connected, a "Trade on" switch
(Paper | Coinbase...) appears above it, per tab (`ChartWorkspace::trading_venue`, never saved: a tab
starts on paper). On a live venue the panel is `ui/broker_panel.rs`, with an amber rule and a
`LIVE · NAME` badge (`ui/broker_badge.rs`; amber is not the buy/sell colour and never blue) that
paper trading never uses. The paper account is only created while the venue is Paper, and paper
overlays on charts are hidden while it is live.

It draws `BrokerHost::snapshot()` and nothing else, so every window shows the same state.

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
refused with a sentence, never traded on a similar market. The ticket (`ui/broker_ticket.rs`, rules in
`model/broker_ticket.rs`) takes the market's rules and the best bid and ask from the broker, not the
chart, and says so. It offers only what the market allows. Text is parsed exactly (`0.001` stays
`0.001`); a value off the market's increment is reported with the nearest valid value and **never
rounded**. The broker's own preview (fees, total, warnings, refusals) replaces the estimate when it
arrives, and a refusal blocks the order.

Review opens a confirmation with the account, market, side, type, size, price, estimated value and
fee. A **market order, an edit and Cancel all always ask**; confirming a limit or stop-limit order, or a
single cancel, is a setting (Settings ▸ Brokers; the defaults ask for orders and do not ask for a
single cancel). The dialog starts with no button focused, so a stray Enter sends nothing. The order
is then handed to the runtime, which records it in the pending journal before it sends it, sends it
once, and shows it as *Sending* / *Unconfirmed* until the broker lists it.

## Editing and cancelling

The pencil edits an open limit or stop-limit order's price and size **in place** (the broker's edit,
previewed first). It is disabled, with the reason, for anything else. Cancel-and-replace is not
offered anywhere. The confirmation says the order can still fill at its old terms until the change
is applied and that a new price may lose its place in the queue.

## Notifications

Orders placed, partly filled, filled, cancelled, expired or rejected produce a message at the foot of
the focused tab and a system notification, once each (the crate's book deduplicates replays, and the
orders already open when a connection starts are not announced). Settings ▸ Brokers turns them off
(`broker.prefs`, local to this Mac).

## Not done yet / not verified

* Chart trading (order lines, the context menu, header BUY/SELL, dragging an order) comes in the
  next change; until then orders are managed from the panel.
* Nothing here has been run against a real account by the author of the change. The mock
  exchange is built from Coinbase's published specification; see the crate's `docs/coinbase.md` for
  what could not be verified.
* The ticket offers good-til-cancelled and fill-or-kill, not good-til-date (it needs a date picker).
  Brackets, TWAP and scaled orders are not implemented.
* Figures use `.` for decimals and `,` for thousands whatever the system locale; typed numbers accept
  `.` or `,` as the decimal separator but never a thousands separator.
* The shared widgets of the paper ticket were not extracted: the live ticket builds from `kit`
  controls, so paper trading is untouched.
