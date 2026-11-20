# Liquidations and taker flow

A docked panel on the right of a tab (toolbar flame button, next to the watchlist toggle) with two
modes, chosen at its top. It listens to the exchanges' public sockets only while a panel is drawn,
and only for the mode on show. Nothing is stored: each record is the last hour, in memory.

- **Liquidations**: how much was force-liquidated in the last minute, five minutes and hour, split
  between longs and shorts, and a tape of the latest ones.
- **Taker flow**: for the asset of the focused chart, what takers bought minus what they sold
  (net flow) over the same spans, a bar per minute for the last half hour, and the same by venue.

**Read the footnote before the totals.** The exchanges do not send every liquidation, so the
numbers undercount a busy moment (below).

## Where things live

| Piece | File |
| --- | --- |
| `Liquidation`, `Side`, `MinSize`, the rolling `Tape` and its `Snapshot` (totals, rows) | `model/liquidation/mod.rs` (pure; tests in `tests.rs`) |
| The sockets and the lease | `service/liquidation/mod.rs`; one module per exchange (`binance.rs`, `bybit.rs`: session and parser) |
| Owning the feed, the per-frame snapshot cache | `host/liquidation/mod.rs` (`LiquidationHost`, one for the app, in `WindowEnv`) |
| The panel | `ui/market_flow/mod.rs` (`show`, `usd`) |
| The tab's side of it, docking | `workspace/market_flow.rs`; the button is in `ui/workspace/toolbar.rs` |

## How it runs

- `LiquidationHost::watch(ctx, minimum)` is called by every drawn panel each frame. It renews a
  3 s `Lease`, starts the feed if there is none (with an empty tape: a record with a gap in it adds
  up wrongly) and returns an `Arc<Snapshot>`, rebuilt only when the tape's revision, the size filter
  or the second changed. Two tabs share one feed.
- `service::stream::Stream` runs each exchange's `net::ws::Session` on its own thread, with a
  watchdog on the lease: when it lapses the watchdog sets the shutdown flag, which every thread
  honours while reading, waiting to reconnect or (Bybit) retrying its market list, so nothing keeps
  retrying for a viewer that left. `is_running` is true only while **every** thread started still
  runs: one that ended leaves a stream short of an exchange, so `watch` replaces the whole feed
  (with an empty tape) rather than serving half of it. Dropping the feed shuts it down at once.
- A burst asks for a repaint at most every 250 ms (`REPAINT_EVERY_MS`); `watch` also asks for one a
  second later, because the totals roll off with time even when nothing arrives.
- The tape has two records. The rows (at most 20,000, the newest 300 listed: `TAPE_ROWS`) are
  placed by **exchange time**, not arrival, because two sockets deliver in their own order. The
  totals are kept by second and are not capped, so a cascade that drops old rows still counts every
  liquidation of the hour; a span may hold up to a second more than its name says. The size filter
  (`MinSize`) only filters the rows.

## The exchanges (verified against the live sockets)

| Exchange | Socket | What it sends | Side |
| --- | --- | --- | --- |
| Binance USDⓈ-M | `wss://fstream.binance.com/market/ws/!forceOrder@arr`, all markets, nothing sent | `e: forceOrder`, `st`, `o`: `s`, `S`, `ap` (average price), `z` (filled size), `T`; at most the **latest liquidation of each symbol per second**. The stream also carries COIN-M contracts (`st: 2`, sized in contracts worth a fixed dollar amount): they are dropped. An order with `z` of 0 has liquidated nothing yet and is dropped. There is no order id, so the same order seen in two snapshots would count twice | The order is a `SELL` when a long is liquidated: `SELL` is `Side::Long`. Not stated on Binance's page; the standard reading |
| Bybit linear | `wss://stream.bybit.com/v5/public/linear`, one subscribe per `allLiquidation.<SYMBOL>` | `data[]`: `T`, `s`, `S`, `v` (size in the base coin), `p` | Documented: a `Buy` is a long liquidated |

- Bybit has no all-markets topic, so the 60 markets with the most 24 h turnover
  (`/v5/market/tickers?category=linear`) are followed, fetched when the feed starts (retried every
  10 s until it works). A newly listed market is picked up at the next start.
- Notional is price × size in the base coin: dollars for the USDT and USDC contracts. Inverse
  contracts are not followed.
- Both statements in the panel's footnote are about this: Binance's one-per-second throttle and
  Bybit's top 60.

## Taker flow

`model/taker_flow` (pure), `service/taker_flow` (`binance.rs`, `bybit.rs`), `host/taker_flow`
(`TakerFlowHost`), the body in `ui/market_flow/flow.rs`. The asset is the focused chart's:
`taker_flow::asset_of(source, symbol)` is its base (Kraken's `XBT` is `BTC`), for the crypto
exchanges only (a one-letter asset such as `S` counts: Binance's and Bybit's `SUSDT` are split on
their dollar quote); a stock or prediction market shows "No flow for this market". Its `{ASSET}USDT`
markets on four venues are followed: Binance spot and USDⓈ-M perpetual, Bybit spot and linear.

| Venue | Socket | Taker side |
| --- | --- | --- |
| Binance spot | `wss://stream.binance.com/ws/<symbol>@aggTrade` | `m` (buyer is the maker) true: the taker sold |
| Binance perp | `wss://fstream.binance.com/market/ws/<symbol>@aggTrade` | same |
| Bybit spot / linear | `/v5/public/spot` / `/v5/public/linear`, `publicTrade.<SYMBOL>` | `S` is the taker's side |

- Notional is price × size in the base coin, taken as dollars. A trade is added to its second of
  **exchange time**; each venue keeps one total per second for the hour, so memory does not grow
  with trading. Net is buy minus sell; the bar under each total is the buy/sell split.
- **One feed per asset.** `TakerFlowHost` keeps an entry for each asset a panel asks about, each
  with its own record, lease and sockets, so two windows on two assets both accumulate. An entry
  nobody asked about for 3 s is dropped (its sockets close), and one whose sockets stopped is
  replaced by a fresh one. A record is never reused for another asset: a socket still reading a
  frame of the old one writes into the record it was handed, which nothing shows any more. Only
  the mode on show keeps its sockets: the other lapses within 3 s.
- **What was observed.** A quiet market and a socket that was not there look alike in the trades,
  so the record also keeps when each venue's socket was connected (`Flow::link`, fed by
  `Session::run_linked` on every connect and drop). A span is whole when all of it was observed
  (2 s of slack): otherwise its total reads "partial" under the bar, its figures in the venue
  table are muted, and the minute bars it touches are faded (not drawn, on a dark cell, for a
  minute no socket was connected). Totals are partial when any venue's is, so a panel just opened
  shows every span partial until an hour has passed.
- A venue that does not list the market (many perpetuals are `1000PEPEUSDT`) simply stays empty;
  the panel says "Waiting for ... trades" only until any venue has traded.
- Flow is not advice: it says who was aggressive, not who was right.

## Not done

- **OKX.** Its `liquidation-orders` channel reports contracts, so a dollar value needs each
  instrument's contract value from the instruments endpoint, and its docs page could not be read
  to confirm the fields. Add it as `service/liquidation/okx.rs` plus an `Exchange::Okx` once
  checked against the live socket (`live_exchanges_announce_liquidations` is the template).
- Liquidations on the chart, a per-asset filter, Bybit inverse, and an MCP tool.
- Taker flow for the quote currencies other than USDT, for OKX, and for the whole market (the
  exchanges have no all-markets trade stream, so it is one asset at a time).
- The panel is not remembered across restarts: a tab starts with it closed, like the Paper panel.
- Not checked against a second source (Coinglass and the like); the figures are the exchanges'.

## Tests

`cargo test --lib liquidation` and `cargo test --lib taker_flow` run the models, parsers (inline
samples), lease and view tests. The live checks, which wait for each exchange or venue to speak:

```bash
cargo test --lib liquidation::tests::live_exchanges -- --ignored --nocapture
cargo test --lib taker_flow::tests::live_venues -- --ignored --nocapture
```

Pictures of the panel (both themes, default and narrow width): `cargo test --lib ui::market_flow --
--ignored` with `DEGENVIEW_GALLERY_DIR` set.
