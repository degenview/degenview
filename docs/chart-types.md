# Chart types

How a chart draws its price data, how the price-based types construct their own bars, and what
Pine scripts, indicators, alerts, strategies, replay and drawings see on each. This is the
reference for `model::chart_type`, `model::synthetic`, `model::chart_look`, `ui::series_render` and
the chart-type parts of the Pine engine.

## Real data, chart bars, script visuals

```text
Provider candles (canonical, never modified)
        │
        ├── standard types ───────────────────────────────▶ drawn as they are
        │                                                    (scripts read the real OHLC)
        └── price-based types ── model::synthetic ──▶ chart bars ──▶ renderer
                                                            │
                                                            └──▶ indicators, Pine scripts,
                                                                 script alerts, request.security

Pine visuals (plotcandle, plotbar, barcolor, plots …) are layers of their own on top of either.
```

* **Canonical candles** are what the provider returned, merged with live updates. Nothing writes
  constructed values back to them or to the caches; switching type never touches the fetch.
* **Chart bars** are the candles for every standard type, and the constructed bars for a
  price-based one. `ChartCard::body` computes them once per change (`constructed_series`) and
  everything on the chart — the price scale, built-in indicators, Pine scripts, the crosshair,
  drawings' time index — reads them. The market's own last price is still the solid tag on the
  price axis; a price-based chart's own last close is a dashed line with an outlined tag.
* The chart type is stored in `ChartAppearance::style` (`ChartStyle`), so it is saved with the
  chart, with saved layouts and in the layout fingerprint, with no second persistence path. Every
  type keeps its own settings. Rows without it read as plain candles.

## Colours

Each type has its own up/down colours (candles: up/down body, border and wick; bars, columns,
Kagi, P&F: up/down; Baseline and HLC Area: their two lines). The chart's saved bullish/bearish
palette is no longer edited anywhere for these charts: it is only what an unset colour follows, so
charts saved with custom colours look the same and the wells show them. What the chart draws beside
its bars (volume bars, the last-price tag, trend-flip markers) follows the up/down colours of the
type on screen (`ChartStyle::up_down`), falling back to the saved palette. A market drawn as a line
(prediction markets, CoinMarketCap) has no chart types and keeps its two "Line up / Line down" wells.
The preview under the Style picker draws the selected type with the chart's own painter code over
sample prices (a price-based type's box/reversal/range size is replaced by one that suits them).

## The types

| Type | Group | Data class | Scripts see | Settings |
|---|---|---|---|---|
| Candles | Standard | standard OHLC | real OHLC | body/border/wick on-off and colours, colour by previous close |
| Hollow Candles | Standard | standard OHLC | real OHLC | colours, body/border/wick |
| Bars | Standard | standard OHLC | real OHLC | thin, tick on/off, colours, colour by previous close |
| High-Low Bars | Standard | standard OHLC | real OHLC | colours, labels |
| Volume Candles | Standard | standard OHLC | real OHLC | candle settings; width follows volume |
| Line / Line with Markers / Step Line | Lines & Areas | standard line | real OHLC | source, colour, width, dash, marker size |
| Area | Lines & Areas | standard line | real OHLC | source, line, fill at the line and at the foot |
| HLC Area | Lines & Areas | standard line | real OHLC | high, low, close line colours, fill |
| Baseline | Lines & Areas | standard line | real OHLC | source, base level (% of height), line and fill colours |
| Columns | Lines & Areas | standard line | real OHLC | source, up/down colours, colour by previous value |
| Heikin Ashi | Price-based | constructed | HA OHLC | candle settings |
| Renko | Price-based | constructed | brick OHLC | box size (traditional / ATR / %), source, wicks, candle settings |
| Line Break | Price-based | constructed | line OHLC | number of lines, candle settings |
| Kagi | Price-based | constructed | segment OHLC | reversal (traditional / ATR / %), source, colours, width |
| Point & Figure | Price-based | constructed | column OHLC | box size, reversal boxes, source, colours, outlines |
| Range | Price-based | constructed | range-bar OHLC | range (traditional / ATR / %), candles or bars |

Switching Candles ⇄ Line ⇄ Bars never changes what scripts and indicators receive. Switching to or
from a price-based type rebuilds them against the other series. A source that draws a line
(prediction markets, CoinMarketCap indices) keeps its line and offers no selector; Volume Candles
is unavailable on a source that reports no volume, with the reason in the menu. A saved type the
source cannot show falls back to candles on screen and is kept for when it can.

## Construction rules

Builders live in `model/synthetic/` and are pure and deterministic. They read the candles oldest
first and return bars with their own indexing; one candle can make several bars or none. Each bar
records the source candles it came from (`first_source`..`last_source`), the open time of the first
and close time of the last, whether it is confirmed, and its shape (`Ohlc`, `Kagi`, `Column`).
The prefix property holds for every builder and is tested: a prefix of the candles gives a prefix
of the bars, which is what makes replay safe and realtime updates well-defined.

Candles do not record the order prices traded in, so "high/low" sources read a rising candle as
open → low → high → close and a falling one as open → high → low → close. **No tick-level parity
is claimed for any type.**

* **Heikin Ashi.** `close = (o+h+l+c)/4`, `open = (prev open + prev close)/2`, first bar
  `(o+c)/2` (the Pine documentation's own `ticker.heikinashi` example), `high/low` widened to the
  body. Computed over every loaded candle, never from the visible edge.
* **Renko.** A brick forms when price travels a full box past the last brick's close (same
  direction) or past its open (reversal, i.e. two boxes from the close). A candle can run several
  bricks. Size: traditional amount, ATR(n) of the whole series as of the newest candle, or a
  percentage of the newest close. Source: close, or the candle's path. Wicks reach the extremes of
  the candles since the previous brick. The first brick is measured from the first price read.
* **Line Break.** Continue when a close passes the last line's end; reverse when it passes the
  whole run of the last *n* lines (three by default), opening the reversal line at the last line's
  open. At most one line per candle.
* **Kagi.** One vertical segment per swing; it reverses when price retraces the reversal amount
  from the segment's end. Thick while breaking the previous peak, thin while breaking the previous
  trough, switching mid-segment at that level.
* **Point & Figure.** Boxes `floor(price/size)`; an X column extends into higher boxes, an O column
  into lower ones; a column reverses after `reversal` boxes against it and the new column starts one
  box in. Each column holds as many candles as it took.
* **Range.** A bar closes when its high-to-low span reaches the range, at `low + range` (up) or
  `high - range` (down); the next bar opens there. The bar short of the range is the live bar.

Confirmed vs live: a bar made while its candle is still forming is unconfirmed, and so is anything
that can still grow (the last Kagi segment, the last P&F column, a Range bar short of its range).
The renderer fades unconfirmed bars; scripts see the last one as the realtime bar (its close time
is `UNCONFIRMED_CLOSE_TIME`). Unconfirmed bars are never persisted; they are rebuilt from candles.

Bar times: a script needs strictly increasing times, but several bricks can come from one candle.
Each bar's time is the open of the first candle that fed it, and a bar whose time would not
increase is nudged one millisecond. That stays inside the candle's own interval and is only an
ordering key; the true source candles are in the bar's source range. A script's
`timeframe.in_seconds()` is the market candles' length (`ExecutionController::with_bar_seconds`),
not read off the bricks' times.

### Where TradingView's rules are undocumented

These are the textbook rules or this app's own choice, not verified against TradingView; none
claims identity:

* the grid alignment of the first Renko brick and the first Kagi / P&F column;
* the ATR box size: this app fixes it from the newest candle, so a new candle can re-size the whole
  series (it is rebuilt, it never mixes sizes);
* where a Line Break reversal line opens;
* how Volume Candles scale width (linear against the visible maximum, floor at 12 %; full width
  without volume);
* Step Line: the value holds until the next bar, then jumps;
* Hollow Candles: equal to the open counts as "above" (hollow), equal to the previous close as
  "above" (green);
* range sizes are in price units, since candles carry no tick size;
* Renko projection bricks, Range phantom bars and tick/trade-based reconstruction are not
  implemented (providers here serve candles, not trades).

## Pine

* `chart.is_standard` is true for every standard type (the script sees real OHLC however it is
  drawn); `chart.is_heikinashi/renko/kagi/linebreak/pnf/range` are true for their type only.
* `open/high/low/close`, history (`close[1]`), `bar_index`, `ta.*` and user-defined series all run
  over the chart bars: `ta.sma(close, 20)` on Renko averages 20 bricks. Changing only a type's
  looks (colours, widths, markers, base level) never reruns a script; changing the type or what
  the bars are built from does.
* `ticker.heikinashi/renko/kagi/linebreak/pointfigure` return the symbol with the construction
  attached (`BINANCE:BTCUSDT@renko(atr,14,false,close)`); the security provider splits it, fetches
  the real candles and builds the bars with the same builders. On a price-based chart, a plain
  `syminfo.tickerid` in `request.security` goes to the provider too, so it returns the real market
  data. Builders given arguments they cannot use return `na`. `ticker.range` does not exist in Pine.
* `plotcandle()` and `plotbar()` are the script's own layers, drawn as candles / OHLC bars on every
  chart type. Values are not changed; contradictory highs and lows are widened to include the open
  and close at draw time (TradingView's exact normalisation is undocumented). `na` in any of the
  four skips the bar. `color`/`wickcolor`/`bordercolor` may vary per bar; `show_last`,
  `editable = false` and `force_overlay` are honoured. A candle output has Body, Wick and Border
  rows in the Style tab, a bar one colour row.
* `barcolor()` colours the main series of the chart from any script (a pane script too), never
  changes values, and `na` leaves the bar alone. Within one script the last call that sets a bar's
  colour wins; between scripts the first in the chart's list wins (the order scripts are applied
  and listed in Scripts). `offset` and `show_last` are applied. The script's colour wins over the
  chart type's own colours for a candle's body and wick (a hollow candle's outline too, and bars)
  and never overwrites the saved palette. A filled candle keeps its usual border colour: this is
  inferred from a TradingView chart where a script dims the bodies and the outlines stay, not from
  documentation.
* Layering: background → scripts behind the chart (`behind_chart = true` or "Send to Back") → main
  series → scripts in front → drawings → crosshair. A script that does not declare `behind_chart`
  draws in front, as before; the Pine reference's default is true, but this app kept the existing
  look. "Visual Order" in the script's context menu (Scripts tab) overrides either way and is saved
  per applied script. Strategy trade markers always stay in front.
* `force_overlay` on `plot`, `plotshape`, `plotchar`, `plotcandle` and `plotbar` sends that visual to
  the main chart from a script with its own pane; the main chart's scale includes it. A `fill()`
  between two forced plots goes with them; a `fill()` between a forced plot and a pane plot stays in
  the pane, where the forced plot is not drawn.
* Style overrides are scoped to one applied script's outputs; two instances of a script never
  share them.

## Strategies, alerts, paper trading, replay, drawings

* **Strategies** on a price-based chart fill at that chart's constructed prices, which no market
  traded at. The report says so (`StrategyReport::chart`, `pine_report::synthetic_fill_notice`).
  TradingView's "standard OHLC fills" option is not implemented. Paper trading never reads the chart
  bars: its reference prices and fills use the market's.
* **Script alerts** run over the same bars as the chart (`ChartCard::pine_feed`), and a change of
  chart type or construction starts them over silently (like a change of source or inputs).
  **Price alerts** are untouched: they watch the market's quotes.
* **Replay** builds from the candles up to the replay position only; rewinding rebuilds from the
  shorter prefix. No future threshold is computed and then hidden.
* **Pagination / warm-up:** constructions run over everything loaded (Heikin Ashi recursion, ATR
  box size, Kagi/Line Break/P&F state). Older history changes the earliest bars slightly, as on
  TradingView; the chart never starts a construction at the visible left edge.
* **Drawings** keep their time/price anchors and are never rewritten by a type change. On a
  price-based chart they sit where the constructed bars around that time are (anchors outside the
  bars extrapolate at the bars' average spacing); price-only levels are unaffected. Switching back
  restores the exact original placement.
* **Crosshair:** standard types read O/H/L/C(/Vol); constructed bars name themselves (Brick,
  Heikin Ashi, Kagi segment from/to and thickness, P&F column from/to and box count), the source
  candles, whether the bar is live, and the market's last price apart; script candles/bars add
  their own OHLC rows.

## Performance

Rebuilding every price-based series from 5 000 hourly candles in a release build
(`cargo test --release -- --ignored --nocapture construction_cost`): Heikin Ashi ≈ 53 µs, Renko ≈ 67 µs,
Line Break ≈ 62 µs, Kagi ≈ 55 µs, Point & Figure ≈ 51 µs, Range ≈ 190 µs. A rebuild happens when
the candles or the construction change (once per realtime tick at worst), never per paint and
never for a restyle. Rendering is limited to the visible bars.

## Testing and visual checks

Builders, look rules, geometry, the Pine semantics (`pine/chart_type_tests.rs`), alerts, replay
prefixes, persistence and the headless painters are unit-tested. Every type was drawn by eye in a
release build, in light and dark, over live Binance candles with a throwaway data folder, including
indicators, a `barcolor()` pane script and a `plotcandle()` overlay on Renko and Heikin Ashi. The
chart-type menu and the Appearance settings page are covered headlessly (they open and lay out for
every type and source) but were **not looked at**: driving the window with synthetic clicks was not
reliable here. **No side-by-side comparison with TradingView was possible**, so geometry, spacing
and the undocumented rules above are unverified against it; fixtures for them need TradingView
reference data.
