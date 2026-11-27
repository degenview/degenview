# Market flow: liquidations, taker flow and order book

A docked panel on the right of a tab (toolbar flame button, next to the watchlist toggle) with three
modes, chosen at its top. It listens to the exchanges' public sockets only while a panel is drawn,
and only for the mode on show. Nothing is stored: each record is the last hour, in memory.

- **Liquidations**: how much was force-liquidated in the last minute, five minutes and hour, split
  between longs and shorts, and a tape of the latest ones.
- **Taker flow**: for the asset of the focused chart, what takers bought minus what they sold
  (net flow) over the same spans, a bar per minute for the last half hour, and the same by venue.
- **Order book**: the focused chart's market as its exchange's book and trades (below).

**Read the footnote before the totals.** The exchanges do not send every liquidation, so the
numbers undercount a busy moment (below).

## Where things live

| Piece | File |
| --- | --- |
| `Exchange`, `Market` and `Venue`: which exchange and market a feed is about, shared by all three modes | `model/exchange/mod.rs` |
| `Liquidation`, `Side`, `MinSize`, the rolling `Tape` and its `Snapshot` (totals, rows) | `model/liquidation/mod.rs` (pure; tests in `tests.rs`) |
| The sockets and the lease | `service/liquidation/mod.rs`; one module per exchange (`binance.rs`, `bybit.rs`: session and parser) |
| Owning the feed, the per-frame snapshot cache | `host/liquidation/mod.rs` (`LiquidationHost`, one for the app, in `WindowEnv`) |
| The panel | `ui/market_flow/mod.rs` (`show`, `usd`); one file per mode body (`flow.rs`, `book/`) |
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

## Order book

The DOM: the focused chart's market on **its own exchange**, as the exchange's book and trades.
Read-only: nothing here places an order. These are the spot books of **Binance** and **Bybit** and
the book of a **Binance USDⓈ-M perpetual** (`BTCUSDT.P`); any other chart shows "No order book".
`market_of(source, symbol)` decides, from the `InstrumentId`.

### Layers

One concern to a layer, and an exchange is a plug-in at each. A venue supplies its own `BookSync`
(`DepthVenue::book_sync`), so the protocol is stated once and `DepthRecord`, the host and the panel
never name an exchange.

To add an exchange (OKX, say), for the order book:

- `model/depth/sync/<x>.rs` and its export in `sync/mod.rs`, only if its update protocol is not one
  of the two there (a diff stream with ids, or snapshot and deltas); otherwise reuse that `BookSync`.
- A variant of `model::exchange::Exchange`, with its `name`.
- `service/depth/<x>.rs`, a `DepthVenue` (session, tick size, `book_sync`, REST snapshot if needed,
  parser), and its one line in `venue::for_exchange`.
- The `DataSource` that charts use for it, in `market_of` (`model/depth/market.rs`).

The new variant is also an exhaustive `match` in `service/taker_flow/mod.rs`, and `Venue::ALL`
(which sizes the taker flow table) lists every venue, so those two places need a decision even when
the exchange has no taker flow; liquidations are one module per exchange in `service/liquidation/`
and are not required. Nothing else shared is edited.

| Layer | What it holds |
| --- | --- |
| `model/depth/` (pure) | `Price` (an exact `i64` of 1e-8, parsed from the exchange's decimal text, so no float keys), `Book`, `Tape` (the latest 300 trades; traded volume by price, the 2,000 prices nearest the market), `PullStack`, `group` (row sizes, private), `View` (row size, centre and `rows`, how many the panel has room for), `Ladder` (the rows) and `TapeView` (the trades) inside a `DepthSnapshot { link, tick, ladder, tape }`, `Event`. `Side::of_taker` and `Side::row_of` are the one statement of where a trade and a resting order fold. `DepthRecord` is the single door and the only owner of the `Book`: `apply(event, now)`, `link(up)`, `needs_snapshot()`, `snapshot(view, now) -> DepthSnapshot`, `revision()`. A `BookSync` per exchange protocol (`sync/binance.rs`, `sync/bybit.rs`) only judges each update (`Verdict`: replace, apply, stale, gap); the record acts on it. Which exchange a market is on is `model::exchange` |
| `service/depth/` | `DepthFeed` (one `Stream` thread per market) only routes: tick size, then the socket, then events into the record. `new_record(market)` makes the record, with the venue's `BookSync` (`for_exchange(..).book_sync()`). What differs is behind the `DepthVenue` trait (`session`, `tick_size`, `book_sync`, `snapshot`, `parse`) in `binance.rs` and `bybit.rs`, picked by `venue::for_exchange`; `snapshot` is fetched only while the record `needs_snapshot()`, and a venue whose socket sends its own answers `DepthError::Unsupported` and is never asked; `DepthError` (`Http`, `Parse`, `UnknownSymbol`, `Unsupported`) is the look-ups' error; `levels.rs` is the book side shape both send |
| `host/depth/` | `DepthHost::watch(ctx, market, view) -> Arc<DepthSnapshot>`: lifetimes and caching only, on `host::lease_map`; it knows no sync rules (the record comes from `new_record`) |
| `ui/market_flow/book/` | Draws a `DepthSnapshot` (`ladder.rs`, `tape.rs`, `toolbar.rs`; the viewer's choices in `state.rs`). It never sees a `Book`, a socket or an exchange beyond its name |

The viewer's choices (`BookViewState`: row size, scroll, big-trade threshold) are not data: the tab
turns them into a `View` (row size, centre, and how many rows the panel has room for) and hands it to
the host, which caches the snapshot by (record revision, second, view). A new market starts over
(`BookViewState::follow`), because its prices have nothing to do with the last one's.

The model builds exactly `View::rows` rows, so it knows nothing of the screen. The ladder is drawn
after the host is asked, so the panel measures how many lines fit (`body height / row height`, at
least one; the spread line takes the place of the last row when it is in view) and keeps that in
`BookViewState::rows` for the next frame: a resize shows with a frame's delay (the panel asks for a repaint when the count changed), and the first
frame asks for a default.

### The exchanges (verified against the live sockets)

| Exchange | Socket | Book | Trades |
| --- | --- | --- | --- |
| Binance spot | `wss://stream.binance.com/stream?streams=<s>@depth@100ms/<s>@aggTrade` (combined: each message is `{"stream","data"}`; nothing sent) | REST `/api/v3/depth?symbol=&limit=5000` (`lastUpdateId`, 5,000 levels a side, 8-decimal strings) for the start; `depthUpdate` diffs with `U`, `u`, `b`, `a` | `aggTrade`: `p`, `q`, `T`, `m` (buyer is the maker: the taker sold) |
| Binance perp | two sockets: `wss://fstream.binance.com/public/stream?streams=<s>@depth@100ms` (depth answers only under `/public`) and `…/market/ws/<s>@aggTrade` (trades only under `/market`; the taker-flow socket). A wrong path connects and stays silent. A drop of the trade socket leaves the book | REST `/fapi/v1/depth?limit=1000`; diffs also carry `pu`, the previous diff's last id, and `ChainedDiffStream` follows that chain (the ids jump, so the spot range rule would see gaps). Tick size from the futures market list the search holds (`exchangeInfo` takes no symbol) | `aggTrade`, as spot, not wrapped |
| Bybit spot | `wss://stream.bybit.com/v5/public/spot`, one subscribe for `orderbook.200.<SYMBOL>` and `publicTrade.<SYMBOL>` | `snapshot` then `delta`, `data.u` the update id, `data.b` and `data.a` | `publicTrade` rows: `T`, `S` (the taker's side), `v`, `p` |

Tick size: Binance `/api/v3/exchangeInfo?symbol=` (`PRICE_FILTER.tickSize`; HTTP 400 for an unknown
symbol), Bybit `/v5/market/instruments-info?category=spot&symbol=` (`priceFilter.tickSize`; an empty
list for an unknown one).

### Keeping the book right (`BookSync`)

A `BookSync` keeps only its own state (update ids) and judges; `DepthRecord` places the levels, and
feeds pull/stack from the levels the book says changed.

- **Binance.** The socket is opened, then the snapshot fetched (the socket's frames wait in its
  buffer meanwhile), then frames are read. A diff with `u <=` the snapshot's id is dropped; the first
  one used must straddle it (`U <= id + 1 <= u`); each later one must start where the last ended
  (`U == previous u + 1`). Anything else empties the book and asks for a snapshot again, inside the
  frame handler: the feed never ends its session over a gap, because ending it ends the feed for
  good. A diff before any snapshot is a gap too.
- **Bybit.** A `snapshot` replaces the book, and so does any message with `u == 1` (Bybit restarting
  its book; the parser turns it into a snapshot whatever `type` says). A `delta` applies only on top
  of a snapshot and only if its `u` is newer. Bybit sends a snapshot after every (re)connect, so
  none is fetched.
- **A dropped socket** empties the book (`DepthRecord::link(false)`): the ladder says "Reconnecting"
  rather than show a book that is out of date, and shows again once the next snapshot is in.
- Levels are kept to the nearest 2,000 a side (`MAX_LEVELS`), so memory stays bounded.

### The ladder

- **Rows.** A row spans a *group* of ticks, picked from the tick × 1, 5, 10, 50, 100, 500, 1,000 (the
  toolbar's menu; `Ladder::group_choices`), or by default the smallest whose rows cover about half a
  percent of the mid over 40 rows (`group::automatic`, whatever the panel's height). Bids fold
  **down** to the row at or below, asks **up** (`Side::row_of`), so a row is bid or offered, never
  both. A trade folds where its taker hit
  (`Side::of_taker`): a buyer lifts an ask (up), a seller hits a bid (down). A `Ladder` holds the
  rows the panel has room for, the middle one holding the centre price; the panel draws them all.
- **Columns.** Stack, Bid, Price, Ask, Traded (a bar split buy and sell, scaled to the busiest
  row). The spread sits above the best bid; the last trade's row is outlined in its taker's colour.
  Hover a row for what its sizes are worth. Scrolling over the ladder moves its rows; the target
  button in the toolbar centres it on the market again.
- **Pull/Stack.** The net change in resting size at a price over the last 10 seconds: positive is
  stacked, negative pulled. Size that left because a trade took it is not a pull (each trade is
  counted back in), so a level that is eaten reads as zero. A figure is coloured once it is a
  quarter of what rests there (all of a pulled level). It is approximate: a trade and the book
  change that follows come on different sockets and can fall in different seconds.
- **Traded** is since the first trade the record saw, which is when the panel was opened on the
  market (the footnote says since when). It is not history.
- **Tape.** The latest 300 trades, newest first; one of at least $10K, $100K (default) or $1M
  notional is set heavier and tinted by its taker.
- A market the exchange does not list (the tick size answers "unknown symbol") says so; the feed
  keeps asking every 10 s.

The footnote says what a size is: the base coin, price × size taken as dollars for the tape.

## Not done

- **OKX.** Its `liquidation-orders` channel reports contracts, so a dollar value needs each
  instrument's contract value from the instruments endpoint, and its docs page could not be read
  to confirm the fields. Add it as `service/liquidation/okx.rs` plus an `Exchange::Okx` once
  checked against the live socket (`live_exchanges_announce_liquidations` is the template).
- Liquidations on the chart, a per-asset filter, Bybit inverse, and an MCP tool.
- Taker flow for the quote currencies other than USDT, for OKX, and for the whole market (the
  exchanges have no all-markets trade stream, so it is one asset at a time).
- The order book for Bybit perpetuals (no chart is one), for the other exchanges (OKX, Kraken,
  Coinbase, Gate.io, MEXC), and any click-to-trade.
  Binance's book depth beyond 2,000 levels is dropped, and the pull/stack figures are not checked
  against a second source.
- The panel is not remembered across restarts: a tab starts with it closed, like the Paper panel.
- Not checked against a second source (Coinglass and the like); the figures are the exchanges'.

## Tests

`cargo test --lib liquidation`, `cargo test --lib taker_flow` and `cargo test --lib depth` run the
models, parsers (inline samples), lease and view tests. The live checks, which wait for each exchange or venue to speak:

```bash
cargo test --lib liquidation::tests::live_exchanges -- --ignored --nocapture
cargo test --lib taker_flow::tests::live_venues -- --ignored --nocapture
cargo test --lib service::depth::tests::live_books_fill -- --ignored --nocapture
```

The last one starts a feed for BTCUSDT on each exchange, waits for a live book and a trade, prints
the best bid and ask, and then checks for 10 s of diffs that the book never crosses.

Pictures of the panel (both themes, default and narrow width, and the opened menus of the order
book): `cargo test --lib ui::market_flow -- --ignored` with `DEGENVIEW_GALLERY_DIR` set.
