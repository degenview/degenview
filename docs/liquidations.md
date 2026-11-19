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
- Each exchange runs a `net::ws::Session` on its own thread. A thread ends at its first message
  after the lease lapsed; `LiquidationFeed::is_running` then goes false and the next `watch`
  starts a new feed. Dropping the feed sets the shutdown flag. No panel drawn, no socket.
- A burst asks for a repaint at most every 250 ms (`REPAINT_EVERY_MS`); `watch` also asks for one a
  second later, because the totals roll off with time even when nothing arrives.
- The tape keeps the last hour and at most 20,000 events (`Tape::push`); the panel lists the newest
  300 (`TAPE_ROWS`). The size filter (`MinSize`) only filters the rows: the totals count everything.

## The exchanges (verified against the live sockets)

| Exchange | Socket | What it sends | Side |
| --- | --- | --- | --- |
| Binance USDⓈ-M | `wss://fstream.binance.com/market/ws/!forceOrder@arr`, all markets, nothing sent | `e: forceOrder`, `o`: `s`, `S`, `ap` (average price), `z` (filled size), `T`; at most the **latest liquidation of each symbol per second** | The order is a `SELL` when a long is liquidated: `SELL` is `Side::Long`. Not stated on Binance's page; the standard reading |
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
