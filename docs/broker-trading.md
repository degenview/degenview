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

## Not done yet / not verified

See the pull request descriptions of the stack for the current list. Nothing here has been run
against a real account by the author of the change.
