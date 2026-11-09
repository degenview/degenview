# Market profiles: the engine under Volume footprint, TPO and Session volume profile

Three chart types (Volume footprint, Time price opportunity, Session volume profile) draw what a
candle cannot say: who traded at which price, and how often a price was visited. This page is the
shared engine and its data. The chart types themselves are documented as they land.

Code: `src/model/market_profile/` (pure; the footprint builder is `footprint.rs`), `src/ui/series_render/footprint.rs` (painter), `src/workspace/profiles.rs` (per-chart computation), `src/service/binance_trades.rs` (aggTrades and the live
stream), `src/workspace/profile_hub.rs` (fetches and shares the data).

## What TradingView documents (sources: its Help Center pages on the three chart types)

- **Footprint.** Aggregated-intrabar by default: an intrabar candle is a buy when it closed above its
  open, a sell below, and a doji takes the previous candle's side. The intrabar resolution gets
  coarser with age. *Tick-based* calculation (off by default, needs tick data, up to 1D) takes the
  exchange's aggressor side, else the position against the bid/ask, else the tick direction; it uses
  at most 3 million ticks and warns when they do not cover the session. Aggressor side exists on
  BINANCE, COINBASE, BITSTAMP, HYPERLIQUID, CME, CBOT, NYMEX, COMEX and EUREX.
- **Imbalance.** A buy imbalance compares a row's buy volume with the sell volume of the row below,
  a sell imbalance a row's sell volume with the buy volume of the row above; the test is
  `max >= percent / 100 * min`, default 300 %.
- **TPO.** Blocks of 5, 10, 15, 30 (default), 60, 120 or 240 minutes get letters A to Z, then a to z,
  repeating. The value area starts at the point of control and repeatedly adds the single row above
  or below with more blocks (tie: closer to the point of control, then the higher), up to 70 %.
- **Session volume profile.** One profile per session; a bar's volume is spread over the rows its
  range overlaps; rows default to 24; the value area is 70 %; custom sessions are `HHMM-HHMM[:days]`.

## What is here

| Piece | Role |
| --- | --- |
| `price_grid` | Rows are **integer tick indices** on a global grid. A price within rounding noise of a boundary is in the row above it (`0.1 + 0.2` is row 3 of a 0.1 grid). A range needing more than 1000 rows is refused, never truncated. |
| `value_area` | Point of control and value area for volume (the existing classical two-row method, shared with the volume-profile drawing and Pine's `request.footprint()`) and for TPO counts (TradingView's documented single-row method). |
| `sessions` | Day, week (Monday start) and month periods; custom `HHMM-HHMM[:days]` windows cut at their intersections; blocks and letters. UTC or a zone from `pine::timezone` (daylight saving aware). |
| `trades` | `Trade` with the exchange's aggressor side; `TradeBook` is sorted, **deduplicated**, bounded, and knows how far back it is **gap free**. |
| `flow` | `FlowKline` (a candle plus the exchange's taker-buy volume) and `CandleBook` (deduplicated by open time, bounded). `Kline` is unchanged. |
| `quality` | `DataQuality`: trade data, exchange buy/sell split, intrabar estimate, chart-candle estimate, or unsupported with a reason. |
| `binance_trades` | `aggTrades` history newest first by id, the live `aggTrade` stream, one rate limiter, back-off on 429, no retry on 418. |
| `profile_hub` | One entry per market however many charts name it; candles, tick size, trades; polled once a frame; nothing blocks. |

## Decisions

- **Exact where the exchange says so, labelled everywhere else.** A footprint bar built from trades
  is `TradeDerived`. From candles with Binance's taker-buy volume it is `ExchangeSplit` (the split of
  each candle is exact, which prices inside it traded is spread evenly). From close-versus-open it is
  `Intrabar`. Imbalances are only computed for `TradeDerived` bars: an even spread makes every row of
  a candle look alike, so an imbalance on it would be an artifact (`DataQuality::has_real_rows`).
- **A bar is trade-derived only if the book covers all of it.** Ids are consecutive per Binance market,
  so a gap in the ids is a gap in the data (`TradeBook::complete_from_ms`). A bar is never built from
  some trades and some candles.
- **History is bounded and newest first.** `TRADE_WINDOW_MS` (6 hours), `TRADE_BUDGET` and
  `TRADE_CAPACITY` (1 000 000 each). A busy pair has more trades than that in a day, so older bars
  fall back to candles. The numbers are constants to tune against Binance's real limits (see below).
- **A reconnect never double counts or loses trades.** The book drops a trade it holds; when the
  stream resumes past the newest held id, the hub backfills exactly the missing ids (at most
  `MAX_BACKFILL`; a larger gap stays a gap and the coverage rule handles it).
- **The tick size is the exchange's** where known (Binance `PRICE_FILTER.tickSize`, Coinbase
  `quote_increment`, both from the symbol catalog the search already downloads daily); elsewhere the
  caller infers one from the prices and says so.
- **The data is shared, not copied.** The hub owns `Arc<…Shared>` handles; the computation (later
  PRs) reads them under the lock on its own worker and compares `revision` to know when to redo.
  Cards never hold a copy of a million trades.
- **Replay is the computation's job.** It asks for data up to the replay position. The hub never drops
  or hides what the market did.

## Inferred (TradingView does not say)

- POC tie: nearest the middle of the range, then the lower row (as the volume profile).
- TPO value area: the step that reaches the target is taken whole, so the area may hold more than 70 %.
- Weeks start Monday; `Days(n)` groups from the epoch.
- Overlapping custom sessions are cut at every intersection; touching windows stay separate profiles.

## Not verified

- Binance REST was checked live once (`live_binance_history_is_contiguous_and_newest_first`, ignored by
  default): one minute of BTCUSDT came back as about 1000 trades with consecutive ids, so the 6 hour
  window is on the order of 360 000 trades and 360 requests (about 90 s at the 4 a second limiter).
  The WebSocket stream was **not** run: its parsing and the reconnect/backfill logic are tested against
  fakes and the documented payload. Rate-limit behaviour (429/418) is by the documentation, not observed.
- No holiday or early-close calendar (the app has none); a stock "session" is a time window only.
- Coinbase has a trades endpoint with a side; it is not wired (follow-up).

## Volume footprint

The first chart type on the engine. Selected from the new "Order flow & profiles" group of the
chart-type menu.

**Data flow.** Once a frame `ChartWorkspace::tick_profiles` asks the hub for what each footprint
chart needs (`intrabar_plan`: the finest of 1m, 5m, 15m, 30m, 1h candles that divides the chart's
bar and keeps the newest bars within 60 000 candles, else the coarsest that divides it, and trades
for Binance), computes the set on the work queue when anything it depends on changed (one
computation per chart at a time, newest request wins, completed bars cached), and hands the card a
`ProfileFeed`. The card and the snapshot only draw it. Replay cuts the bars the set is built for, so
nothing after the replay position is read; the current replay bar is shown whole, as its candle is.

**Per bar, best first:** trades of the bar when the book covers all of it (`TradeDerived`), candles
with Binance's buy/sell split (`ExchangeSplit`), candles by TradingView's close-versus-open rule
(`Intrabar`), the chart's own candle (`ChartBars`). The status line at the bottom left of the chart
counts bars per source; a hover shows the bar's source. Imbalances and stacks exist only for
trade-derived bars.

**Drawn:** the candle in the middle of each bar, sell cells on its left and buy cells on its right
(Cluster) or as wide as their volume (Profile); Buy and sell, Delta, Total or Ladder; the four-step
gradient (below 25 %, 50 %, 75 %, above) against the busiest side on screen; the point of control
outlined; the value area shaded with lines at its ends; imbalances as an edge bar (buy right, sell
left); stacked imbalances projected right until a later bar's range enters them; a two-line summary
(delta, volume) under each bar; the hover (price range, buy, sell, total, delta, imbalance, POC or
value area, the bar's totals and its data source). With less room a cell loses its numbers, then
becomes a shaded cell, then the bar is a plain candle: the meaning never changes, only the writing
(`detail`: 11 points tall and 26 wide for numbers, 2 and 3 for cells).

**Automatic row size** is 0.2 × the average true range (length 14) in whole ticks, at least one
(TradingView: "0.2 × normalized ATR"; the length is inferred, its default is not stated). The tick is
the exchange's, else inferred from the prices' decimals and labelled so.

### Inferred (TradingView does not say)

- Gradient scale: the busiest single side of any cell on screen (TradingView says "relative to the
  range"). One scale for every bar in view, so a colour means the same volume everywhere.
- The summary is a two-line text under each bar (delta, volume); TradingView's Info Box and Table are
  not reproduced. The **Table** summary mode and the other summary fields (volume change, min and
  max delta, session CVD, open interest) are **not implemented**.
- Defaults other than the 300 % imbalance and 70 % value area; stacked levels 3.
- Candle in the middle of the bar; TradingView's layout was not compared side by side.
- Tick-based calculation is on by default where the market has trades.

### Not verified

- Never compared against a TradingView chart, and not driven in a real window (headless frames and
  PNG galleries only). The live WebSocket and a real-account flow were not run.
- Bars older than the newest 3000 draw as plain candles; a footprint is not built for them.
- The tooltip placement uses the existing script-tooltip mechanism (egui's own delay and placement).
- A Pine script sees the real candles; whether TradingView does on its footprint charts is not
  verified.
