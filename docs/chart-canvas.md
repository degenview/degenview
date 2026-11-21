# Chart canvas and chart templates

Reference for Chart Settings ▸ **Canvas**, the chart appearance templates, and how they are
built. Companion to [chart-types.md](chart-types.md) (Appearance ▸ chart types).

The model is `src/model/chart/canvas/mod.rs` (`ChartCanvas`, kept in `ChartAppearance::canvas`), the
painting is `src/ui/chart/canvas.rs` and `ChartCard::body`, the page is
`src/ui/chart/settings/canvas.rs`, and templates are `src/model/chart/template/mod.rs`,
`src/store/chart/template/mod.rs` and `src/ui/chart/settings/templates/mod.rs`.

## What is verified against TradingView, and what is not

Checked against TradingView's own pages (Supercharts configuration guide, the Charting Library
chart-overrides and watermark pages, and the "apply the same settings to all charts" article):

| Behaviour | Source |
| --- | --- |
| Canvas has Chart basic styles, Scales, Buttons and Margins sections | Supercharts guide |
| Background is solid or gradient | Supercharts guide; overrides `backgroundType` |
| Vertical and horizontal grid lines each have a colour and opacity | Supercharts guide |
| The pane separator is only shown once an indicator pane exists | Supercharts guide |
| Crosshair has colour, opacity, thickness and style | Supercharts guide; overrides `crosshair.*` |
| The watermark shows the symbol and the interval; it has colour and opacity | Supercharts guide; watermark page (symbol, description, interval) |
| Scales have text size, text colour and line colour | Supercharts guide; overrides `scalesProperties` |
| Navigation buttons: zoom in/out, scroll left/right, reset view | Supercharts guide |
| Pane buttons: move up/down, collapse, maximize, remove | Supercharts guide |
| Dragging the line above a pane resizes it; the pane above (or the main chart) gives up what it gains | TradingView behaviour; the exact grab width and minimum heights are ours |
| Top and bottom margins are percentages | overrides `topMargin`/`bottomMargin` |
| Settings are per chart, and "Apply to all" copies one chart's to the rest of the layout | apply-to-all article |

Not stated by TradingView's documentation, so **inferred**, not claimed as TradingView-exact:

- The defaults: top margin 10 %, bottom 8 %, right margin 10 bars, crosshair dashed, buttons
  "visible on mouse over". They match what the TradingView UI ships with as far as is known;
  there is no authoritative source.
- The option names "Always visible / Visible on mouse over / Never visible" and their meaning.
- The scale text sizes (10, 11, 12, 13, 14, 16, 18 pt) and the margin limits (a margin is at most
  45 %, both together at most 80 %; right margin at most 200 bars).
- A "Replay mode" watermark. TradingView's pages only describe symbol, description and interval
  visibility; DegenView adds "Replay mode" (shown only while a replay runs, hidden otherwise).
- Which of the three line styles each setting offers.

## Settings

Every colour is optional: unset follows the app theme (the swatch shows what it follows), picked
stays put through a theme change until **Reset** hands it back. The picker's alpha is the
opacity. Every row writes straight into the chart, which repaints at once.

- **Background.** Solid, or a vertical gradient (top and bottom). Both fill the plot, its scales
  and every script pane in one rectangle, so there is no seam between them. A gradient is one
  four-vertex mesh the GPU interpolates: no texture to rebuild, exact at any size or display
  scale. The solid colour and the two stops are each remembered while the other mode is on. The
  default gradient bottom is the chart surface leaning towards the accent. Pine's
  `chart.bg_color` still reads the theme's, not the custom backdrop.
- **Vertical / horizontal grid lines.** On/off, colour (with opacity) and solid/dashed/dotted.
  The horizontal lines stand on the price axis' labels and the vertical lines on the time axis'
  labels: one tick list each, from `model::chart::canvas::price_ticks` and `time_ticks`. Prices are
  1/2/2.5/5 × 10ⁿ on a linear scale and 1/2/3/5/7 × 10ⁿ decades on a log scale (thinned to fit).
  Time labels are on every n-th bar counted from the start of the series, so a label stays on its
  bar while the chart pans. The grid is drawn from the scale and never feeds back into it.
  Each script pane has its own price grid on its own scale and shares the vertical one.
- **Pane separators.** The line above each script pane: on/off, colour, thickness, style. Shown
  only when the chart has a pane; with none, the row says so. The line is also the handle that
  resizes the pane below it (see Panes).
- **Crosshair.** Colour, opacity, thickness (1–4 pt) and style. The vertical line runs through the
  price chart and on through every pane; the horizontal line and price tag belong to the chart
  under the pointer. It is drawn above the series, the scripts and the drawings. Restyling does
  not move it. (The crosshair follows the pointer freely; it does not snap to a bar.)
- **OHLC values.** On/off for the box of values in the plot's top-left corner while the pointer is
  over the chart (open, high, low, close, and volume, `Mkt`, constructed-bar and script-candle values
  where they apply). On by default. Off hides the whole box; the crosshair lines, the price and time
  tags and the ring on a line chart's bar stay. The switch is DegenView's own, not stated by the
  TradingView pages cited above. Snapshots never contain the box (it belongs to the crosshair).
- **Countdown to bar close.** On/off for the time the forming bar has left, a second row inside the
  main series' last-price tag on the price axis ([the countdown](#countdown-to-bar-close) below).
  Per chart, in the Scales section of this page and in the price axis' right-click menu. On by
  default (TradingView's Supercharts default could not be established: its charting library's is
  off, the chart the page was checked on had it on). Persisted in the canvas, so templates and
  layouts carry it; a row with the setting on is written without the key, so a build that predates
  it reads and writes the row unchanged.
- **Watermark.** Hidden, Symbol (market and timeframe, with the source) or Replay mode. Centred in
  the plot, sized to the plot (at most 72 pt, never above 70 % of its width or 24 % of its height
  for the title) and left out when it would be under 16 pt. It is painted behind everything the
  chart draws and takes no input.
- **Scales.** Text size, text colour and line colour. The size drives both axes (the time axis is
  one point smaller), the price axis' width, the time axis' height, the price tag height and the
  label spacing, so the geometry follows the font. It does not touch menus or Pine tables.
  The price axis is as wide as the widest label it paints now (grid ticks, the tags' prices and
  every script pane's ticks, measured by `ChartCard::body` before the plot is laid out), plus a
  margin, rounded up to 4 pt (`canvas::axis_width`) and never under 44 pt scaled by the text size.
  Only the visible range counts, so an old high out of view does not widen it. The width changes
  by a step when the range crosses a digit boundary. The 44 pt floor and the 4 pt step are
  inferred, not taken from TradingView.
- **Buttons.** Navigation (bottom-left of the plot: zoom in, zoom out, scroll left, scroll right,
  reset view) and Pane (top-right of each script pane: move up/down, collapse/expand,
  maximize/restore, remove). Each is Always, On hover or Never. The group fades; it counts as
  hovered while the pointer is over it, so it does not vanish under the pointer. The fade state is
  kept per window, since chart ids repeat across windows. A collapsed or
  maximized pane keeps its buttons, they are its way back. Hiding them never disables the wheel,
  trackpad, drag or keyboard: the buttons drive the same viewport.
- **Margins.** Top and bottom are a percentage of the plot's height kept free above the highest
  and below the lowest price, applied in log space on a log scale. Right is a number of bars.
  Fields keep what you type until you leave them or press Return, then clamp to their range (the
  vertical pair may not leave less than a fifth of the height; the other field's room is the
  bound). Unit steppers apply at once.

### The right margin

The plot always has `visible + reserved` equal slots (`model::chart::canvas::visible_window`), where
`reserved` is the larger of the right margin and what an overlay script reserves. At rest the
newest bar is `reserved` slots from the right edge. Scrolling back by `n` bars fills those slots
with the bars that follow, so the bar spacing never changes as you scroll or at the moment the
last bar leaves the margin. Changing it fetches nothing and does not touch Pine's bar indexing
(the run's visible range is margin-independent), and a price-based chart uses its own bars as
slots.

What an overlay script reserves is the farthest slot its bar-anchored drawings reach past the last
bar, capped at 20 slots (`ui::pine::geometry::MAXIMUM_FUTURE_BARS`, the fewest candles a chart
shows), so one line drawn hundreds of bars ahead never leaves the candles less than half the plot.
The cap is one number at every zoom: the zoom gestures and the Screenshot Studio count on the
reserved slots staying put while the number of candles changes. Pan right to see the rest: the pan goes on until two bars are left (see
[Chart navigation](chart-navigation.md)); what lies farther than that needs a zoom out.

### Script ranges follow the bars on screen

The price axis and every script pane fit their range to the script bars the plot shows, wherever
the chart is scrolled (`ui::pine::geometry::VisibleBars`: count, `tail` newer bars off the right
edge, and the empty slots on screen past the newest bar). Series values and drawings (boxes, lines,
labels, polylines) outside those bars add nothing. A pane is drawn across the whole plot, margin
included, so a drawing in the empty slots on screen counts for the pane as for the main chart. A
box that extends, or a horizontal line that does, runs the plot's width: it counts when it crosses
the bars shown (a line's `extend.left` runs past its first point and `extend.right` past its
second, so one written right to left swaps sides) (measured on TradingView, which widens its axis for both). A sloped line counts by
its anchors alone, extended or not: TradingView does not widen the axis for the price it reaches
far from them, at any slope, and a vertical line never extends. The live chart, the
Screenshot Studio, Select portion and the MCP snapshot all read this through
`ChartCard::extent_of`, which sizes the reserved slots first and then lays the bars out again in
the plot they make. Not counted: the quad of a linefill, which is drawn but adds no price of its
own.

### Overlays on `scale.none`

An overlay declared `indicator(..., overlay = true, scale = scale.none)` leaves the price axis
alone: `overlay_extent` takes none of its values, hlines or drawings (it still asks for its
`future_bars`). The renderer maps it on a hidden scale of its own, `own_scale` in
`ui/chart/card/scripts.rs`: `pane_value_range` over the same `VisibleBars` the panes use, widened by
the chart's own top and bottom margins and laid across the band the price scale uses (below the
header). It is linear whatever the price axis is (Percent, Log), ignores a manual price range, and
nothing on screen reads it: no axis labels, no last-value tag. Each overlay's `PineLayer` gets its
own `y_for`, so labels, boxes, lines and their hover targets agree. The values tooltip and the Table
view list the values as they are (`ValueKind::Plain` in the table). `scale.left` and `scale.right`
are still warned about (`PINE7001`: one value axis), and so is `scale.none` on a script with its
own pane, which TradingView only allows with `overlay = true`. Not verified against TradingView:
the margins the hidden scale leaves (we use the chart's) and whether it follows a Percent or Log
price axis (we keep it linear).

### Panes

Pane order is the order of the scripts on the chart (`PineInstance` list), so moving a pane
reorders that list and there is no second representation. Collapse and dragged height are the
instance's pane layout (`PineInstance::pane`, a `PaneLayout`, stored as its `pane_collapsed` and
`pane_share` fields and saved with the chart; the script still runs). Maximize is view state and is
not saved. Remove is the same removal as Scripts ▸ Remove from Chart. Layout is
`model::chart::canvas::split_panes`: an expanded pane has the height it was dragged to
(`PaneLayout::share`, a fraction of the chart's height), or else its
part of the default 30 % of the height divided by the number of expanded panes; a collapsed one is a 24 pt strip, and a
maximized pane takes everything but the strips and the time axis; the price chart then draws nothing
but its time axis. The main chart takes what the panes leave and keeps at least 90 pt (60 % of a
very short chart). Every expanded pane, dragged or not, keeps at least 48 pt, enough for its plot
and its buttons; a chart without room for that gives its panes less, never under 24 pt (the
buttons' height) while their strips fit, and the main chart yields last. What the panes want beyond those floors is cut
back in proportion. `pane_limits` holds these numbers for the split and for the drag alike.

Dragging the line above a pane (`card/pane_divider.rs`, 3 pt either side, resize cursor) moves
height between the two panes it separates, as in TradingView: the pane below gains what the nearest
expanded pane above it (or the main chart) loses, so the other panes and, between two panes, the
main chart keep their size. Neither goes under the floors above. The first drag saves
the heights of every expanded pane; a pane added later takes the default share and the main chart
gives it the room, one removed gives its room back to the main chart. A height follows its script
when panes are moved. Undo and redo of a script change (`workspace::undo`) restore which scripts are
on the chart but keep each script's `pane` as it is now; a script that comes back after a removal has the layout it
was removed with. Collapsed panes and a maximized pane have no line to drag. The drag is
measured from the split it began on (`model::chart::canvas::drag_divider`), so rounding never
accumulates, and every height is a difference of rounded edges, so a pane the drag does not touch
never moves. A drag ends when the chart's panes or height change under it. The line takes the
pointer whatever tool is armed (Zoom In included), as the pane buttons do, but a workspace mode that
owns the whole chart (Select portion, a replay start or Pine Logs range pick) sees no line: no resize
pointer and no drag. There is no double-click reset.

The built-in indicators (volume, EMA, Bollinger, trend flips) are drawn inside the price plot, not in
panes, so they have no pane buttons. RSI is not one of them: it is the bundled Pine indicator `rsi`,
in a pane like MACD (see [Indicator catalog](indicator-picker.md)).

## The colour picker

Every swatch (`kit::color_well`) opens `ui/kit/color_picker/mod.rs`, one picker drawn with the painter in
the app's own style, so it is identical on macOS, Windows and Linux (no egui stock picker, and no
native `NSColorPanel`: no crate wraps the three platforms' dialogs, and Windows' has no opacity).
It has a saturation/brightness square, hue and opacity sliders (opacity only where the setting
has any), a hex field (`#RRGGBB`, `#RRGGBBAA`) and an opacity percent field, the macOS system
colours and the last ten picked (kept with egui's persisted memory). Hue, saturation and
brightness are plain HSV of the sRGB bytes, so halfway across the square is halfway on screen. The
hue is remembered while the popup is open, so dragging through black or grey does not lose it.
Controls take the arrow keys (Shift for ten times the step) and carry accessibility labels.
Not offered: an eyedropper and a numeric RGB/HSB mode.

## Layering

Back to front: backdrop → grid → watermark → Pine "behind chart" visuals → main series →
indicators and Pine foreground → drawings → replay overlay → last-price tag → time axis →
script panes and their separators → crosshair → paper-trading markers → navigation buttons.
`barcolor()`, `plotcandle()`, `plotbar()`, tables, drawings, alert levels and replay overlays are
drawn exactly as before.
Script label and table-cell tooltips are not a layer: the live chart asks egui for one over the
hovered label or cell after the tables are drawn (see the Pine guide), and a snapshot has none.

## Countdown to bar close

`model::chart::countdown` (pure, given the time) and `ChartCard::draw_last_price`.

- **What it counts to.** The close of the bar that holds *now* on the bucket grid every source cuts
  its candles on (`bucket::next_bucket_start_ms`), not the newest candle's open plus a length, so it
  rolls over to the next bar by itself while the next candle is still on its way and nothing is made
  up. While the newest candle is the forming bar its own `close_time` must be where the grid puts it
  (within a second), else the countdown is hidden: provider and grid disagree. It is a function of the
  clock alone (a missed frame, sleep, or a clock set back change nothing but the instant asked).
- **When it is hidden**, rather than mislead: the setting is off; the type is price-based (Renko,
  Kagi, Line Break, Point & Figure, Range: `ChartType::is_time_based`; Heikin Ashi counts down with
  its candle); the source has no clock-cut bars (`DataSource::counts_down_bars`: not line charts,
  CoinGecko, nor Alpaca, whose bars follow a US session with holidays and early closes that nothing
  models: a countdown would run through a closed market); a replay cuts the chart (`cut_ms`);
  nothing is loaded for the timeframe (`Chart::data_timeframe`); or data has not arrived for three
  refresh intervals of the source (arrival of a refresh or the live feed, not of a trade, so a
  quiet market is not a dead one; a failed refresh alone does not hide it). Both are recorded by
  `Chart::note_arrival`, which a refresh and an accepted live bar call alike, so a chart the socket
  keeps current counts down while its REST answer is slow or failing.
- **Text** (`countdown::format`): `MM:SS` under an hour, `HH:MM:SS` under a day, `4d 3h` from a day
  (hours rounded up), never negative.
- **Tag.** The price row stays on the price line; the countdown is a second row of the same filled
  tag, in the same size as the price, a line (1.2 × the price size) under it, or over it where the
  plot ends too soon for the row. Both rows start at one left edge, the wider of the two centred in
  the tag, as TradingView lays them out (measured on its canvas: equal glyph heights, rows 14 CSS px
  apart, left-aligned). The text is black on the greens and light fills and white on the reds and
  dark ones (`on_color`, shared with the other axis tags), since white on the green is hard to read.
  Comparison-overlay tags keep clear of the whole tag (`TagBand`): the room they have is the plot
  less the tag, each keeps its side, and one its side has no room for goes to the other, so the
  plot's edge never pushes a tag back over the countdown. The axis is
  measured with the widest text the timeframe can show (`widest_texts`) whenever the setting is on
  and the chart's type and source are eligible, whether or not the countdown is showing, so its width
  never changes with the digits or with the connection.
- **Frame.** The workspace resolves it once per frame (`Chart::bar_countdown`, one clock reading for
  all charts) and pushes it into the card (`ChartCard::set_countdown`), like the replay cutoff. The
  card never reads the clock; the Pine Studio preview has no data age and shows none. A repaint is
  asked only at the next change of the text (`Countdown::next_repaint`), by drawn charts only.
  Nothing is stored, fetched, recomputed or marked dirty by a tick.
- **Snapshots** include it, frozen at the capture (`ChartParts::countdown`, resolved once in
  `chart_parts`), so previews and exports of one capture agree.

Checked on TradingView (BTCUSDT, 2026-11-25): the toggle is *Labels ▸ Countdown to bar close* in the
price scale's right-click menu; a 5m bar read `01:57`, a 1D bar `02:36:39`, a 1W bar `4d 3h` and a
1M bar `5d 3h`; replay (1D) shows no countdown. Its support page lists the types above, delayed data
and a market that is not trading as reasons to show none. **Inferred, not verified:** seconds count
down to `00:00` (floor); where the price sits relative to the price line (here on it, the
countdown below); its place above the price near the plot's bottom; Heikin Ashi; what TradingView shows in the instant a bar has
closed and the next has not arrived (here the next bar's count); the intraday replay (paid plan).

## Templates

A template is a named, frozen copy of a chart's `ChartAppearance`: chart type and every type's
settings, colours and the canvas. It holds no symbol, source, timeframe, data, indicators, Pine
instances or inputs, alerts, drawings, paper positions, replay position, geometry or chart id
(a test checks the stored keys). The price axis' decimals are not applied: they follow the
market, not the look.

- **Menu** (chart settings, beside the Reset button of Appearance and Canvas): Save current settings as template…, the apply scope
  (*Apply appearance and canvas* / *Apply colors and canvas only*), My templates, Presets,
  Manage Templates…, Apply current settings to all charts…, Reset chart appearance to defaults….
- **Colors and canvas only** is DegenView's own addition. It keeps the target's chart type and
  what its bars are built from (brick/reversal/range sizes and sources, line count), checked
  against `Construction::of` for every type, so its data does not move.
- **Applying** goes through `ChartCard::apply_template`; a different chart type goes through
  `set_chart_type`, which resets the scroll and rebuilds the constructed bars exactly as the
  toolbar's chart-type menu does. Nothing is replaced wholesale, so scripts, indicators, drawings and alerts
  stay.
- **Presets** (Default Dark, Default Light, Minimal, High Contrast, Presentation) are built from
  code, never stored, cannot be changed and can be duplicated into editable templates.
- **Manage**: apply, duplicate (`<name> copy`, numbered), rename (with description), update from
  this chart (asks first, never silent), delete (asks first), created/updated time.
- **New charts** start from one template, chosen with the star on its row in Manage (a preset
  can be chosen; the star again goes back to the app's own look). The choice is the setting
  `chart.defaultTemplate`, an id, so updating or renaming the template changes what new charts get,
  and deleting it clears the choice. It applies where a chart is added (Add Chart, ⌘K, a
  watchlist row, MCP `add_chart`, a new tab), through `ChartWorkspace::new_chart`, with the same
  rule as Apply to all: a market that cannot draw the template's chart type, or is drawn as a
  line, takes its colors and canvas only (`scope_for_new_chart`). Charts that already exist, saved
  layouts and duplicated charts keep their own look, and *Reset chart appearance* still returns to
  the app's own defaults. A Mac that has the choice but not yet the template (iCloud) starts
  charts from the app's look until it arrives.
- **Names** are trimmed, 1–48 characters, no control characters, unique ignoring case (presets
  included); descriptions are up to 200 characters.
- **Storage**: the `chart_template` document table (`id/position/payload`). The sheet reads it
  when it opens; every change re-reads the table first, validates, then writes in one
  transaction, so two open sheets cannot overwrite each other. A row this version cannot decode
  stops all writes to the table and the sheet says so; presets keep working and the row is left
  alone. Nothing is read from the database while painting.
- **Apply to all** (`ChartWorkspace::apply_appearance_to_all`) plans every chart first
  (`plan_apply_to_all`), then applies. It restyles the other charts of that workspace only (not
  other workspaces or saved layouts) and reports how many. A chart whose market cannot draw the
  look's chart type keeps its own type and takes colours and canvas (counted in the message); a
  market drawn as a line has no types and is not counted.
- **Reset**: *Reset Canvas to Defaults* (Canvas page) puts the canvas back and leaves colours and
  the chart type alone. *Reset chart appearance* (Templates menu) resets the type and its
  settings, colours and canvas; it keeps indicators, scripts, drawings and the axis decimals.

## Known differences from TradingView

- No per-pane background, grid or scale text settings; one canvas serves every pane.
- The crosshair does not snap to bars or have TradingView's Magnet modes.
- The watermark has no separate symbol/description/interval toggles or custom text.
- Only solid/dashed/dotted line styles (TradingView's library has more).
- Pine `chart.bg_color`/`fg_color` follow the app theme, not a custom backdrop.
- Charts are per-chart settings only; there is no document-wide "apply to every window".
- No toggles for the last-price label or line (TradingView has *Symbol last price label* and a
  line one); only the countdown can be turned off.
- The crosshair's price pill and a drawing's price tag paint over the last-price tag, countdown
  row included, as they do over the price alone.
- No countdown for stocks, forex or other session markets, and none for CoinGecko.
- Not visually compared against TradingView side by side (no reference available offline); the
  rows above say which rules are documented and which are inferred.
