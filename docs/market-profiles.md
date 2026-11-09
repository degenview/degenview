# Market profiles: the engine under Volume footprint, TPO and Session volume profile

Three chart types (Volume footprint, Time price opportunity, Session volume profile) draw what a
candle cannot say: who traded at which price, and how often a price was visited. This page is the
shared engine and its data. The chart types themselves are documented as they land.

Code: `src/model/market_profile/` (pure; the footprint builder is `footprint.rs`), `src/ui/series_render/footprint.rs` (painter), `src/ui/series_render/tpo.rs` and `svp.rs` (the TPO and session-profile painters), `src/workspace/profiles.rs` (per-chart computation), `src/service/binance_trades.rs` (aggTrades and the live
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
  PNG galleries only).
- Run once against **real Binance** (`live_binance_footprint_end_to_end`, ignored by default): the
  chart's candles, the 1-minute candles with the taker-buy split, the exchange tick size (0.01 for
  BTCUSDT), the trade history and the live `aggTrade` stream all arrived through the real hub; seven
  bars came out `ExchangeSplit`, the current hour `TradeDerived`, and the live stream kept adding trades
  to the book (the newest id advanced between two looks 5 s apart). That is one run on one market, not
  a soak: reconnect/backfill against the real exchange, a long session and rate-limit refusals were not
  exercised.
- Bars older than the newest 3000 draw as plain candles; a footprint is not built for them.
- The tooltip placement uses the existing script-tooltip mechanism (egui's own delay and placement).
- A Pine script sees the real candles; whether TradingView does on its footprint charts is not
  verified.

## Time price opportunity (TPO)

The second chart type on the engine: a letter profile per day, week, month or custom session. Pure
engine in `model/market_profile/tpo.rs`, painter in `ui/series_render/tpo.rs`, settings in
`ui/chart_style_settings/tpo_rows.rs`.

**Data.** Always **5-minute candles** (every block size TradingView offers, 5, 10, 15, 30, 60, 120 and
240 minutes, is a multiple of five), fetched by the hub as `5m` from the oldest period the candle
budget reaches (60 000 candles: about 208 days of days, 29 weeks, 6 months). They are never taken from
the chart's own bars, so the chart's timeframe does not change the profile. A bar of the chart that no
profile covers stays a plain candle, and the status line counts the profiles.

**Built:** every candle marks each price row its high-to-low reaches with the letter of the block it
opened in. A row's count is the number of blocks that visited it. A price exactly on a row boundary is
in the row above it, so a candle whose high is the bottom of a row has visited that row.
Point of control = most blocks; **value area from block counts** (TradingView's single-row steps,
`value_area::tpo`), never volume; initial balance = the first `n` blocks (default 2); poor high / low =
the extreme row holds more than one block; single prints = inner rows with one block; rotation factor,
total / above / below POC counts; open, close, high, low. The **volume profile** beside it is a
separate measure: each candle's volume spread evenly over its range, its own POC and value area.

**Periods.** Day, week (Monday to Sunday) and month groups of `n` in UTC, or up to five custom
`HHMM-HHMM[:days]` windows (UTC; overlapping windows are cut at their intersections). Block 0 starts at
the span's start, so a split profile counts its blocks from the split.

**Split and merge** are `TpoEdit`s kept per chart (`Chart::tpo_edits`, saved in the layout as
`tpo_edits` and left out while empty), replayed over the periods the settings give: a split cuts the
profile holding an instant, a merge joins the profile starting at an instant to the one before it, and
an edit that names nothing does nothing. The right-click menu on a **letter** offers *Split profile at X*,
*Merge with previous profile* and *Reset all merges and splits* (`Target::TpoBlock`). Changing the period,
the periods per profile, the block size or the sessions drops them (TradingView's rule); a layout's saved
edits survive the first look. They are not part of a template and not in the undo stack.

**Replay and the clock.** The computation reads only candles that open before the end of the last
displayed bar, and a span that has not ended by the clock is *forming*; a forming profile shows only the
blocks that have happened and its levels are not extended.

**Drawn.** Letters packed from the profile's start (or in the column of their block with *Expand
blocks*), coloured by a four-colour gradient (the first two span A to Z, the last two a to z), rows
outside the value area at a configurable opacity; with less room letters become coloured blocks, then one
bar per row (the same data, less writing). Levels as lines across the profile (POC, VAH, VAL, midpoint,
open, close, poor high / low, single prints, initial balance as a bracket on the left), POC / poor
high / poor low / single prints optionally carried right until a later bar's range reaches them. The
volume profile sits beside the letters (left or right, with optional values and POC / VAH / VAL lines).
Hover: the row's range, count and letters, the block's time (UTC), the row's volume, the profile's
POC / VAH / VAL and block count.

**Automatic row size** is TradingView's documented formula: the range of the newest 300 bars in ticks,
over 80 rows, rounded to a step (5 up to 100, 50 up to 1000, 500 up to 10 000, and so on), at least one.

### Inferred (TradingView does not say)

- POC tie: nearest the middle of the range, then the lower row. The step that reaches the value-area
  target is taken whole.
- The **TPO midpoint** is the middle of the profile's high and low.
- Letters are packed left to right in block order; *Expand blocks* puts each in its block's column.
  How TradingView lays them out was not compared.
- Where the volume profile and the lines sit, the default colours and the defaults (initial balance of
  2 blocks, volume profile on at the right, poor highs, poor lows and single prints on).
- "Extend until intersected" means the first later bar whose high-low range reaches the level; only
  finished profiles are extended.
- Weeks start on Monday; `n` days group from the epoch.

### Not done / not verified

- **Summary info** (the pane of per-profile values: HL range, VA range, rotation factor, ...) is not
  implemented (the values are computed; there is no pane). Session presets other than All and Custom
  (Each, pre-market, market, post-market) are not offered: there is no extended-hours metadata. Custom
  windows are in UTC, not the exchange or chart time zone; there is no holiday calendar.
- Split and merge are not in the undo stack; alerts on TPO levels are not implemented.
- Run once against real Binance (`live_binance_tpo_end_to_end`, ignored by default): four days of
  BTCUSDT 5-minute candles gave four profiles of 48 half-hour blocks (the current day forming with 43),
  with levels and a picture that looked right. Not compared with TradingView, not driven in a real window.

## Session volume profile

The third chart type on the engine: a horizontal volume histogram per session. Engine in
`model/market_profile/svp.rs`, painter in `ui/series_render/svp.rs`, settings in
`ui/chart_style_settings/svp_rows.rs`.

**Data.** Lower-timeframe candles, the finest of 1, 5, 15, 30 or 60 minutes whose candles for every
session in view fit the 60 000-candle budget (`candle_plan`: twenty days of minutes, a hundred days of
5-minute candles, and so on; failing that hourly candles reaching as far back as the budget allows).
Never the chart's own bars. TradingView supports intraday and daily charts only, and so does this:
weekly and longer charts show "unavailable: the chart's bars are too long" and fetch nothing.

**Built.** Each candle's volume is spread over the rows its high-to-low overlaps in proportion to the
overlap (TradingView's documented rule), so the total is conserved. Up and down: a candle with
Binance's taker-buy split contributes it exactly (the profile is then `ExchangeSplit`); any other is up
when it closed above its open, down below, a doji following the candle before it (`Intrabar`, an
estimate, said in the status line). POC and value area use the volume method (70 %). **Rows:** *Number
of rows* (24) picks ticks per row as the session's range in ticks over the rows, rounded, at least one,
on the global price grid, so a session can have a row more than asked (TradingView says extra rows may
be added; its own worked example, 100 ticks with 30 rows giving 3 ticks and 34 rows, is a test);
*Ticks per row* is as it says.

**Developing levels** (POC, value-area edges) are the levels *as they stood* after each candle, from the
volume that had traded by then and nothing later: there is a test that the level after the first `n`
candles equals the point of control of a profile built from only those `n` candles. They are worked out
only when a setting asks for them, at most 400 points a session. (The row size of a finished session
comes from its whole range, so the rows are the same at every step; the volume in them is not.) Replay
reads only candles before the end of the last displayed bar, and a session not over by the clock is
*forming*: its lines are not extended.

**Drawn.** Up/Down (stacked from the placed edge, up first), Total or Delta bars; up/down colours with
separate value-area colours (outside the value area at half strength); the POC, VAH and VAL lines
across the session, each optionally carried right until a bar crosses it; the developing lines as step
lines; row values; width as a share of the session; left or right placement; a translucent histogram
box. Thin rows (under 2 points) are merged into buckets of the **average** volume per row, so the
shape is the same at any zoom. The candles of a covered session are not drawn (as in TradingView); a bar
no profile covers stays a plain candle. Hover: the row's range, volume, up, down, delta, value-area
membership, the session's span (UTC), its POC/VAH/VAL, totals and where its numbers came from.

### Inferred (TradingView does not say)

- Defaults other than 24 rows, 70 % and "extend off": width 30 %, left placement, POC/VAH/VAL shown,
  developing lines off, box opacity 12 %.
- Up/down for a candle without an exchange split follows TradingView's per-candle rule for volume
  profiles (close against open); a doji follows the candle before it.
- The developing value area is the value area of the volume so far (the same method, no lookahead).
- The "extend" line stops at the first later bar whose range crosses it, and only for finished sessions.
- Colours: teal and red, the value-area colours following them until set.

### Not done / not verified

- Session presets Each, Pre-market only, Market only and Post-market only are not offered: the app has no
  extended-hours metadata and no exchange calendar (stocks are not supported at all here). Custom windows
  are in UTC. There is no "custom session time zone" and no holiday handling.
- TradingView's `max_total_rows` (about 6000) is not mirrored: the bound is 400 sessions and 1000 rows a
  session. Its "profiles align to the start of the year" statement is not reproduced.
- Run once against real Binance (`live_binance_svp_end_to_end`, ignored by default): four days of
  BTCUSDT through the real hub gave four sessions from 1-minute candles with the exchange's buy/sell
  split (25 rows each, 360 developing points), and a picture that looked right. Not compared with
  TradingView, not driven in a real window.
