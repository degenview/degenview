# Liquidations

A docked panel on the right of a tab (toolbar flame button, next to the watchlist toggle): how
much was force-liquidated in the last minute, five minutes and hour, split between longs and
shorts, and a tape of the latest ones. It listens to the exchanges' public liquidation sockets
only while a panel is drawn. Nothing is stored: the record is the last hour, in memory.

**Read the footnote before the totals.** The exchanges do not send every liquidation, so the
numbers undercount a busy moment (below).

## Where things live

| Piece | File |
| --- | --- |
| `Liquidation`, `Side`, `MinSize`, the rolling `Tape` and its `Snapshot` (totals, rows) | `model/liquidation/mod.rs` (pure; tests in `tests.rs`) |
| The sockets and the lease | `service/liquidation/mod.rs`; one module per exchange (`binance.rs`, `bybit.rs`: session and parser) |
| Owning the feed, the per-frame snapshot cache | `host/liquidation/mod.rs` (`LiquidationHost`, one for the app, in `WindowEnv`) |
| The panel | `ui/liquidation/mod.rs` (`show`, `usd`) |
| The tab's side of it, docking | `workspace/liquidations.rs`; the button is in `ui/workspace/toolbar.rs` |

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

## Not done

- **OKX.** Its `liquidation-orders` channel reports contracts, so a dollar value needs each
  instrument's contract value from the instruments endpoint, and its docs page could not be read
  to confirm the fields. Add it as `service/liquidation/okx.rs` plus an `Exchange::Okx` once
  checked against the live socket (`live_exchanges_announce_liquidations` is the template).
- Liquidations on the chart, a per-asset filter, Bybit inverse, and an MCP tool.
- The panel is not remembered across restarts: a tab starts with it closed, like the Paper panel.
- Not checked against a second source (Coinglass and the like); the figures are the exchanges'.

## Tests

`cargo test --lib liquidation` runs the model, parsers (inline samples), lease and view tests. The
live check, which waits up to four minutes for each exchange to announce one:

```bash
cargo test --lib liquidation::tests::live_exchanges -- --ignored --nocapture
```

Pictures of the panel (both themes, default and narrow width): `cargo test --lib ui::liquidation --
--ignored` with `DEGENVIEW_GALLERY_DIR` set.
