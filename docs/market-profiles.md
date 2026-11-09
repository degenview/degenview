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

- **A bar is built from trades only if the book reaches both ends of it.** It must start before the bar (no gap since) and hold a trade at or after the bar's end (the last bar is still forming and is exempt), so a closed bar that spans a dropped connection stays an estimate until the gap is backfilled.
- **Replay asks for the right candles.** A chart in replay far from now tells the hub where its needs *end* (`ProfileNeed::to_ms`), so the finer candles around the replay position are fetched, not the newest 60 000; stepping forward fetches what lies after what is held. Older history arriving into a full candle book makes room by dropping the newest, not the candles just fetched.
- **Backfills queue.** Every skipped id range of a batch is kept and backfilled one after another.
- **A doji follows the candle before it within its bar**; the first doji of a bar is a buy (TradingView compares with the previous intrabar, which may be in the previous bar: not reproduced).

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

## All three together

### Data sources: what is exact and what is estimated

| Market | Volume footprint | Time price opportunity | Session volume profile |
| --- | --- | --- | --- |
| Binance | **Trade-derived** (exact per price) for bars the recent trade window covers (up to 6 h or 1 M trades, newest first, plus live); older bars **ExchangeSplit** (each 1-minute-or-finer candle's buy/sell exact, rows estimated) | Exact on 5-minute candles | Up/down **exact per candle** (taker-buy), rows spread evenly: `ExchangeSplit` |
| Coinbase | Estimated (close vs open on candles); ~60 days of 5-minute candles | Exact on 5-minute candles (~60 days) | Estimated up/down; same depth |
| DEX (GeckoTerminal) | Estimated; 180 days, hourly paging only | Exact on 5-minute candles | Estimated up/down |
| Alpaca (stocks) | Unsupported (no intraday candles wired, IEX-only volume) | Unsupported | Unsupported |
| CoinGecko, prediction markets, CMC | Unsupported (no volume or candles) | Unsupported | Unsupported |

Nothing estimated is presented as exchange data: each footprint bar and each session profile carries a
`DataQuality`, shown in the status line and the hover.

### Settings inventory

- **Volume footprint:** tick-based calculation, row size (auto or ticks) and ATR length, display
  (cluster, profile), type (buy and sell, delta, total, ladder), gradient, buy and sell colours, value
  area on/off and percent, point of control, summary (info box, off), imbalance percent, highlight,
  colours, stacked highlight and levels, the candle's own settings.
- **Time price opportunity:** period (days, weeks, months) and count, block size (7 sizes), row size and
  ticks, value area percent, sessions (all, custom ×5), initial balance blocks, four gradient colours,
  blocks, letters, opacity outside the value area, expand blocks, ten levels each with show, colour and
  (where it applies) extend, the volume profile (show, placement, values, POC/VAH/VAL, two colours).
- **Session volume profile:** sessions (all, custom ×5), volume (up/down, total, delta), value area
  percent, rows layout and size, POC/VAH/VAL each with show, extend and colour, developing POC, developing
  value area, values, width, placement, four colours, histogram box colour and opacity.

Every control changes what is built or drawn; none is decorative. Unavailable ones are disabled with the
reason (tick-based calculation on a market with no trades) or not offered (session presets without
extended-hours data).

### Compatibility

- **Pine and indicators.** The three types are `DataClass::StandardOhlc` and construct nothing
  (`Construction::of` is `None`, `chart.is_*` stays standard), so scripts, indicators, `request.security`,
  script alerts and panes read the provider's own candles exactly as on a candle chart. Tests:
  `the_order_flow_types_construct_nothing…`, `a_script_alert_on_an_order_flow_chart…`, and the snapshot
  test with indicators, drawings and replay on. Whether TradingView feeds scripts the same on its own
  footprint charts was not verified.
- **Drawings and alerts.** Pinned to time and price through the chart's own projection; the painters
  never move or hide them, and the snapshot drift test runs with a drawing on. Price alerts read quotes,
  never a profile level; alerts on TPO or profile levels are not implemented.
- **Replay.** Each computation reads only bars up to the replay position (and, for the TPO and session
  profile, candles that open before the end of the last displayed bar): footprint bars, TPO blocks and
  developing levels never show the future (prefix properties are tests). The current replay bar is shown
  whole, as its candle is, and there is no sub-bar stepping.
- **Snapshots.** One renderer (`paint_frame`); a test compares live and snapshot shape for shape for each
  type *with its data*, and that taking a snapshot changes nothing.
- **Persistence.** Per-type settings ride in `ChartStyle` (`#[serde(default)]`), kept per type, in layouts
  and templates (what a profile is built from stays the target's); the TPO's splits and merges are saved
  with the layout. An older build cannot read a layout that uses the new types (documented).
- **MCP.** `degenview_set_chart_type` lists the three types and refuses unsupported sources;
  `degenview_get_candles` still returns the market's own candles.
- **Menu.** The type menu has a new group, scrolls only when the window is shorter, and is keyboard
  navigable (arrows, Enter).

### Compared with TradingView

Implemented from its documentation and **checked by tests against those documented rules**: the
imbalance test and its defaults, the four-step gradient, aggregated-intrabar classification, letters and
block sizes, the TPO value-area steps, the TPO row-size formula, the session-profile row rule and its own
worked example, 70 % value areas, the initial balance, poor highs/lows, single prints, "extend until
intersected". **Not verified:** any pixel or layout comparison (no TradingView chart was compared side by
side), the exact letter layout, the footprint cell layout, defaults TradingView does not state, tick-based
calculation beyond Binance, the summary panes and tables, session presets, and how scripts behave on its
own footprint charts. Do not read "similar to TradingView" as parity.

### Measured

Release builds, one machine (Apple silicon), made-up but realistic data
(`cargo test --release --lib market_profile::bench -- --ignored --nocapture`, and
`series_render::bench` for frames):

| Work | Time |
| --- | --- |
| Insert 300 000 trades into the book | 2.6 ms |
| Footprint, 500 hourly bars (30 000 one-minute candles, 5 trade-derived bars), cold | 12.6 ms |
| Footprint, nothing changed / one live trade | 1.7 ms / 1.4 ms |
| TPO, 120 daily profiles from 34 560 five-minute candles, cold | 14.4 ms |
| TPO, nothing changed / the newest candle revised | 0.3 ms / 0.3 ms |
| Session profiles, 200 sessions, cold: without / with developing levels | 11.6 ms / 22.9 ms |
| Session profiles, nothing changed | 0.5-0.7 ms |
| One frame (shapes plus tessellation), 2400 x 1400 points: footprint, 40 bars and 3 038 cells with numbers | 1.8 ms |
| One frame: footprint with coarse rows / TPO with 19 909 letters / session profile with developing lines | 0.2 ms / 2.8 ms / 0.2 ms |

Work runs on the work queue, never the UI thread, and the hub's shared data is only `try_lock`ed on the
UI thread. Not measured: memory use, the network fetches (the history of a day of Binance trades is
hundreds of requests at 4 a second), repeated type switching, replay rewinds, snapshot generation time.

### Looked at

Pictures of each painter in the dark theme (wide, medium, narrow, short, each option) and the light theme
(`series_render::gallery`) were looked at; the light ones read fine except the volume profile's point-of-control
line, which was white and is now the theme's text colour. Real-data pictures of each type came from the
ignored live tests. Not looked at: a real window, a high-DPI display other than the 2x the gallery uses.

### Known limitations

Exact footprints only on Binance and only in the recent trade window; the footprint is built for the
newest 3000 bars; weekly and longer charts have no session profile; no extended-hours session presets, no
holiday calendar, custom sessions in UTC; summary panes and tables, alerts on levels and undo for TPO
splits are not implemented; Alpaca and other non-crypto markets are unsupported; nothing was compared with
TradingView or driven in a real window.
