# Forecasting, volume and measurement tools

Twelve drawing tools in a fifth flyout off the strip, the **Forecast** tile, in the three sections
TradingView gives them: FORECASTING AND MEASUREMENT, VOLUME-BASED and MEASURERS. They are drawings
like every other (placed, previewed, selected, moved, reshaped, restyled, hidden by timeframe,
locked, undone, saved, synced, listed in the Object tree, drawn in snapshots): a tool is a row of
`Tool::spec`, a `Geometry` variant and a projection. This page is what is specific to them.

Three things hold for all of them:

- **A drawing never trades.** A Long or Short position models a *hypothetical* trade. Creating,
  moving or deleting one places, changes and cancels nothing; no order, paper or live, is made from
  it, and nothing the paper-trading engine or the alert engine runs reads a drawing (a test pins
  that: their sources do not mention drawings). Its stats say "hypothetical". The account it is sized
  against is the one typed into its settings, never the paper account's balance.
- **Forecasts are pictures.** A Ghost feed's candles and a Bars pattern's copy are drawn on top of the
  chart. They are never in the series, so they reach no indicator, script, alert, backtest or
  order, and are never fetched, cached or saved as market data.
- **Numbers come from prices and bars, not pixels.** Every measurement uses the stored prices and
  the bars the chart draws (constructed bars on a Renko chart, and only what a replay has shown), so
  it is the same on a linear, logarithmic, percent or indexed axis and a replay never sees a bar it
  has not revealed.

Provenance, as in [Drawing tools](drawing-tools.md): **doc** is stated by a TradingView help page,
**lib** by the Charting Library's drawing overrides, and **inferred** is chosen here because nothing
found says. Nothing was compared pixel for pixel against TradingView (no access).

## Tools

The Forecast tile shows the last tool used from its menu; its corner, a right-click or the Right arrow
open the menu. Tools of fixed points are placed like every other: click-click or press-drag for the
first two, a click for each more; Backspace takes the last back, Esc or a right-click abandons it.

| Tool | Placement | What is drawn | Default (lib unless noted) |
|---|---|---|---|
| Long position | one click (**doc**: "a click at the entry point") | a green zone above the entry and a red one below, the entry, target and stop levels across them, stats tags, the three prices on the axis | 1 px `#808080` lines; zones `rgba(8,153,129,.2)` and `rgba(242,54,69,.2)`; white 12 px stats |
| Short position | one click | the same turned over: profit below, stop above | as Long |
| Position forecast | 2: source, target (**doc**: Price 1 the source, Price 2 the target) | an arrow from source to target, the source's price, the target's move and time, and Success or Failure once the bars say | `#2962FF` 2 px; success `#4CAF50`, failure `#F23645` |
| Bars pattern | 3 (**inferred**): the first and last bar to copy, then where to set it down | the copied bars as bars, candles or a line, from the anchor | `#2962FF`, mode Bars, not mirrored or flipped |
| Ghost feed | 2: start, end (**doc**: "click anywhere ... set the starting point") | hypothetical candles on the bars between them | up `#ACE5DC`, down `#FAA1A4` at 50 %, borders `#089981` / `#F23645`, wick `#808080` |
| Sector | 3 (**doc**: an origin, a point "to the right in the future", a point "at an estimated price") | a wedge of a circle: border and background | not in the library: `#2962FF`, background 20 % (**inferred**) |
| Anchored VWAP | one click (**doc**) | the VWAP from that bar to the latest, with bands | `#1E88E5` 1 px, source HLC3, bands at 1, 2 and 3 deviations (only the first on, **inferred**) |
| Fixed range volume profile | 2 (**doc**) | a histogram of volume by price over the bars between them | 24 rows, Up/Down, value area 70 %, 70 % wide from the left edge |
| Anchored volume profile | one click (**doc**) | the same from that bar to the latest | as above |
| Price range | 2 (**doc**) | a box between two prices with an arrow and a label | `#2962FF` 2 px, background 60 % transparent |
| Date range | 2 | a box between two times over the plot's height | as above |
| Date and price range | 2 | a rectangle between the anchors with both arrows | as above |

## Positions

The formulas are TradingView's published ones ("How to use long and short position drawing tools",
**doc**), in `model/position.rs`. Entry, stop and target are prices; `Side` carries the direction.

- Risk size: the risk percentage of the account, or the amount typed.
- Quantity (in lots): the smaller of the **risk quantity** `risk size / (stop distance x point value) / lot`
  and the **leverage quantity** `account x leverage / (entry x point value) / lot`, rounded to the
  quantity precision. TradingView words the second as "divided by entry price, times point value, divided by
  lot size"; with a point value of one the readings agree, and this one keeps a quantity's unit. The
  point value is one: no market here has a contract multiplier to follow.
- Profit at the target is `distance x quantity x point value x lot`, the loss at the stop the same with
  its distance; the **Amount** on each tag is the account after it.
- Reward over risk: `(target - entry) / (entry - stop)` for a long, with the entry first for a short.
- A stop on the wrong side of the entry (above a long's) has no risk to size: no quantity, no reward
  over risk, no money; the tags and middle say `—` and the distances stay honest. A target on the wrong
  side is a loss and is shown as one. Nothing is NaN or infinite.
- **Ticks** are steps of the last place the chart prints the price to (`PriceFormat::tick_size`: `1` for 67,432,
  `0.01` for 12.34). This build has no market's real tick size, so these are not an exchange's.
- **How it stood**: the first bar after the entry bar to reach the stop or the target decides (a bar that
  reaches both counts as the stop, **inferred**: the bars cannot say which came first). Reached, the
  middle shows `Closed P&L` in the success or failure colour; otherwise `Open P&L` to the last close.
  Only bars the chart has are read, so a replay knows only what it has shown.
- A new position is sized from what is on screen: the target a tenth of the plot's height from the
  click, the stop half that on the other side, the right edge a fifth of the plot's width on
  (**inferred**: TradingView does not say how it sizes one). The target and the stop share the right edge, which
  is the drawing's width (`Geometry::constrain` keeps the stop's time under the target's).

Stats show while the position is selected (**doc**: "always show stats" off means so); "Always show
stats" and "Compact stats" are settings, and so are the account (default 1,000), lot (1), risk (25 %
of the account or an amount), leverage (1) and quantity precision (**lib** defaults). The tags are
`Target: 10.00 (10.00%) 1,000, Amount: 1,100`, `Stop: ...` and a middle plate with
`Long (hypothetical)`, the P&L, the quantity and the reward over risk.

**Position forecast** is the source, the target and what happens between. It shows the source's price
and the target's price change, percentage, bars and time; once the bars have reached the target it
says Success, and once the time has run out without that, Failure (**inferred**: TradingView says the forecast "will
automatically be deemed a success or a failure" and not how). It claims no probability.

## Bars pattern

The three clicks choose the first and last bar to copy (shown as a date range while the second is
placed) and where to set the copy down; it follows the pointer until the third click. (The click order
is **inferred**: TradingView's help for the tool does not give placement.)

- **The copy is kept in the drawing**, as `[open, high, low, close]` offsets from the first bar's
  close, at most 500 bars (a longer selection keeps its first 500). A drawing that read the chart
  again would change when older history pages in, when the chart type changes, and on another Mac that
  does not hold those bars. It is a few kilobytes: well inside a sync record.
- It is set down with the first bar's close on the anchor, one bar to a slot, rightwards. Price offsets are
  absolute, not scaled.
- **Mode** (bars, candles, or a line through close, open, high, low or HL/2), **Mirrored** (reflected
  vertically) and **Flipped** (run backwards in time, each bar's open and close swapped) are settings
  of the drawing; the help says only that "Mirrored/Flipped" reflect "vertically or horizontally", which
  is which is **inferred**, and that the mode is bars or a line on different price sources.

## Ghost feed

Hypothetical candles from the first price to the second, on the bars between the two anchors, whichever
was placed first (the run is read from the earlier to the later; "backward" is placing it from the
right). TradingView says only that it "uses previous data as well as your inputs"; the candles here are
made from the two anchors, two numbers and a stored seed, never from market data:

- A random walk pinned at both ends, so the last candle closes exactly on the second price; each
  candle opens where the one before closed.
- A candle's range is the **average high-low** times `1 +- variance`, never smaller than its body;
  the wicks share what is left. The average is in percent of the first price, or at zero (the
  default) **the average range of the 20 bars before the feed starts** ("previous data"). TradingView
  sizes it in ticks, which this build has no real size for.
- The same inputs and seed draw the same candles every time; "Draw another run" in the settings
  changes the seed (and so is one undo step and one synced record). At most 500 candles.

## Sector

Three clicks (**doc**): an origin, a point to its right that sets the radius, and a third that sets
the angle. The wedge runs from the origin out to the radius along the first ray, round the circle the
short way to the second ray (the third point's direction), and back. The help says nothing of the
construction beyond the clicks, so this is **inferred**. Style is the border and the background (**doc**).

## Measurers

Price range, Date range and Date and price range read two anchors. The label prints what is on
(**doc**: price, percent, pips or ticks, bars, elapsed time, volume; all on by default, **lib**):

- **Price**: the change in price (`+12.00`), its percentage of the start (`+12.00%`) and its steps (`+1,200`)
  of the chart's last printed price place (see Ticks above).
- **Bars** counts both ends (`5 bars`) from the bars the chart draws; **time** is the elapsed time
  between the anchors, which is not the bars (a market that is shut at weekends has no bars there).
- **Volume** is the volume of those bars, only when the market has any.

A price range is a box over the width of the drag, a date range a box over the plot's full height, and a
date and price range the rectangle between the anchors. The label hangs beyond the box on the side the
price went (a date range's sits in the middle); a note of the user's sits above it.

## Anchored VWAP

`VWAP = sum(price x volume) / sum(volume)` from the anchor bar (included) to each bar. The source is
HLC3 by default (**lib**) and any of the chart's price sources. The bands are the VWAP plus and minus a
multiple of the **volume-weighted standard deviation** `sqrt(sum(volume x price^2) / sum(volume) - VWAP^2)`; the
library names three multipliers (1, 2, 3) and a mode, Standard Deviation. It runs to the latest bar and
**does not reset at session boundaries** (**inferred**: the page does not say). A bar before any volume has
traded has no VWAP; a market with no volume has none at all, and the drawing says so (`No volume data`) instead
of drawing a line it made up. A bar that is not a number is skipped.

## Volume profiles

`model/volume_profile.rs`, and one engine for both tools: they differ only in which bars they cover (two
anchors, or an anchor to the latest bar).

- The rows span the lowest low to the highest high of the bars, as a number of rows (default 24, at
  most 500) or rows of so many steps of the chart's last printed price place (TradingView's "Ticks Per Row").
- **Each bar's volume is spread evenly over its low to its high**, each row taking the share its prices
  overlap; a bar with no range puts all of it in one row. A bar is **up volume when it closed above its open**
  and down volume otherwise (**doc**: "the close > open condition is used"). Volume mode is Total, Up/Down
  or Delta (up less down, in the colour of the side with more).
- The **point of control** is the row of most volume; two equal rows are told apart by nearness to the middle
  of the range, then the lower (**inferred**). The **value area** starts at the point of control and grows by the heavier
  pair of rows beside it (two above against two below; one if only one is left; a tie grows upward) until it
  holds the value area percentage (70 by default, **doc**) of the volume. This is the classical market-profile
  method: TradingView does not document its own (**inferred**), and the area is not simply the middle 70 % of the price range.
- Drawn: bars from the range's left edge (or its right, leftwards) up to 70 % of its width; rows outside
  the value area dimmer; the point of control (optionally run on to the edge of the chart) and the value area's
  high and low as lines.

**What the data can and cannot say.** A bar carries one volume and a range, not the trades inside it. TradingView
refines a profile with lower-timeframe bars (down to one second, stepping through 1, 5, 15, 30, 60, 240 minutes and a day until
the range holds fewer than 5,000 bars); this build has no bars under an hour, so a profile here is a distribution of
the chart's own bars and must not be read as tick-accurate. Volume is not comparable across sources (CoinGecko,
Polymarket, Kalshi and CoinMarketCap send none; Binance and Coinbase send base volume, GeckoTerminal quote volume;
Alpaca's feed is IEX only), so a profile's absolute volume is not labelled, only its shape. A range that starts before the
oldest bar the chart has loaded covers the loaded bars only, and says so (`From the loaded bars only`): scrolling
back pages older bars in and the drawing follows.

Profiles and the VWAP are computed when the shape is built, from the bars the chart draws, over the bars in range
(each bar touches only the rows its range overlaps), so they are cheap enough to rebuild without a background job; the
shape is cached and rebuilt when the bars change. The cache key reads the newest bar's **volume** too: a live bar whose
volume alone changes used to leave a cached shape stale.

## Settings

The Inputs tab of each tool holds what is specific to it (the account and risk, the result colours, the mode and
reflections, the candle size and colours, the source and bands, the rows and value area, what a measurer prints); every
control is one the drawing honours. The floating toolbar offers a position's compact and always-on stats, a VWAP's
source, a profile's volume mode and a bars pattern's mode and reflections. All of them are part of the drawing's style
(or, for a bars pattern's reflections and a ghost feed's seed, its geometry): undone, saved and synced with it,
and kept in a template.

## Storage and sync

Each tool is a `Geometry` variant (`Position`, `Forecast`, `Bars`, `Ghost`, `Sector`, `Vwap`, `FixedProfile`,
`AnchoredProfile`, `Measure`); a position keeps its entry, target and stop as three anchors; the VWAP and the
profiles read only the *time* of their anchors (`Geometry::reads`), so the Coordinates tab does not offer the price
the handle was left at. The options are left out of a stored drawing while they hold their defaults. A bars pattern with
no bars or a number that is not a number is a damaged row: it is kept as `Drawing::Unreadable`, never drawn. Drawings sync
as part of saved layouts, one record each: nothing in the sync engine changed, and an older build keeps one as unreadable and
syncs it back untouched ([newer versions](icloud-sync.md#newer-versions)). Mocked-sync tests run all twelve across two Macs,
move a position's stop and draw another ghost run (each uploads one record), and the database keeps them exactly.
**Real CloudKit and two real Macs were not exercised.** MCP lists the new tools read-only
(`long_position`, `anchored_vwap`, ...) and can delete them; it cannot create or edit them.

## Not done, approximate or unverified

- **Nothing was compared with TradingView**; every visual (label plates, zone and candle colours, arrow heads, the profile's
  look) is chosen here or taken from the Charting Library's defaults.
- **Inferred**: the Bars pattern's placement clicks and its mirror and flip; the Ghost feed's candle model; the Sector's construction;
  the Position forecast's success rule; how a new position is sized; a double-hit bar counting as the stop; the value area
  and point of control methods; the profile's width, placement and colours; the VWAP not resetting and its default bands;
  ticks as the chart's last printed place.
- **Volume is an approximation of the chart's own bars** (above), with no lower-timeframe data; a profile or a VWAP of a market
  with no volume says so.
- **No account integration**: the account size is typed, never the paper account's; no "trade this" action, since
  no order-from-drawing flow exists to reuse (the only drawing-to-action path is an alert at a horizontal line's price, which
  opens the alert editor without saving).
- **No contract specifications**: no tick size, lot size, multiplier or quote currency is known for any market, so a
  position's point value is one and its amounts carry no currency.
- **Not offered**: a position's reverse action, its alerts, "Open P&L" of a position drawn in the future in TradingView's sense
  (here: to the last close), the volume profile's developing lines and value labels, anchored VWAP's other band modes, a measurer's
  extend flags, creating or editing these over MCP.

## Tests

`model/{measure,position,volume_profile,ghost,bars_pattern,forecast_style}.rs` have hand-worked fixtures: a long and a
short worked end to end (quantity, profit, loss, balances, reward over risk), the leverage cap, rounding, a stop or target on
the wrong side, nothing NaN whatever the inputs, the first level reached and the double-hit bar, a forecast read only from
the bars it is given; a VWAP and its deviation from a table of prices, no volume, a bad bar; a profile's spread, its up and
down split, the point of control and its tie-break, a value area grown pair by pair, the row sizes, the volume conserved
for every row count; the ghost run pinned at both ends, deterministic, never a candle that is not one; a bars pattern's
copy, its cap, its two reflections and that they commute. `ui/drawing_layer/forecast_tests.rs` projects every tool on the
100 px a bar, 10 px a price plot of the line tests and checks each position, label text and colour by hand, including the
same texts on a logarithmic scale, a position's zones as its hit area, a profile's bar and the gap beside it, a range that
starts before the loaded bars, and that a live volume change rebuilds the cached shape. The every-tool loops of the drawing
layer (placement, undo, Escape, select, move, handles, extreme anchors, save and reopen, a Renko chart, a log scale) run all
twelve, as do the model, store and mocked-sync round trips and the snapshot comparison with the live frame.

## Manual visual test matrix

Compare against TradingView in both themes at 1x and 2x. The hand-run gallery
(`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_forecast -- --ignored`) writes every tool as a PNG from the app's own
renderer; it is not a comparison with TradingView.

| Check | Where |
|---|---|
| Zones, levels, tags and the three axis prices; compact stats; stats only while selected | Long and Short position |
| A position placed with one click, then dragged by its target, its stop and its entry | Long position |
| Open, target-reached and stop-reached middles | a position over a volatile chart |
| Source, target and result labels; Success and Failure | Position forecast |
| The copy following the pointer after the second click; mode, mirrored, flipped | Bars pattern |
| Candle size at several zoom levels; another run; drawn from the right | Ghost feed |
| Bands on and off, the fill, a source change; a chart with no volume | Anchored VWAP |
| Rows, up/down/delta, left and right, the point of control extended; a range before the loaded bars; scrolling back | profiles |
| Labels' readability over dense candles in both themes | measurers |
| The Forecast menu: width, section headings, scrolling in a short window, keyboard | the tile's corner |

## Sources

TradingView help: [Drawing tools available on TradingView](https://www.tradingview.com/support/solutions/43000703396-drawing-tools-available-on-tradingview/),
[How to use long and short position drawing tools](https://www.tradingview.com/support/solutions/43000475660-how-to-use-long-and-short-position-drawing-tools/),
[Long position](https://www.tradingview.com/support/solutions/43000517002-long-position-drawing-tool/),
[Short position](https://www.tradingview.com/support/solutions/43000516992-short-position-drawing-tool/),
[Position forecast](https://www.tradingview.com/support/solutions/43000517004-position-forecast-drawing-tool/),
[Bar pattern](https://www.tradingview.com/support/solutions/43000517006-bar-pattern-drawing-tool/),
[Ghost feed](https://www.tradingview.com/support/solutions/43000748168-ghost-feed-drawing-tool/),
[Sector](https://www.tradingview.com/support/solutions/43000516995-sector-drawing-tool/),
[Anchored VWAP](https://www.tradingview.com/support/solutions/43000669764-anchored-vwap-drawing-tool/),
[Fixed range volume profile](https://www.tradingview.com/support/solutions/43000707985-fixed-range-volume-profile-drawing-tool/),
[Anchored volume profile](https://www.tradingview.com/support/solutions/43000707989-anchored-volume-profile-drawing-tool/),
[Date and price range](https://www.tradingview.com/support/solutions/43000516996-date-and-price-range-drawing-tools/).
Charting Library: the override interfaces `RiskrewardlongLineToolOverrides`, `RiskrewardshortLineToolOverrides`,
`PredictionLineToolOverrides`, `BarspatternLineToolOverrides`, `GhostfeedLineToolOverrides`, `AnchoredvwapLineToolOverrides`,
`DaterangeLineToolOverrides` and `DateandpricerangeLineToolOverrides` for the defaults. The help pages were read through a
summarising fetch, not in a browser, so a detail a page states only in an image is missing from this page.
