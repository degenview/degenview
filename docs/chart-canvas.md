# Chart canvas and chart templates

Reference for Chart Settings ▸ **Canvas**, the chart appearance templates, and how they are
built. Companion to [chart-types.md](chart-types.md) (Appearance ▸ chart types).

The model is `src/model/chart/canvas/mod.rs` (`ChartCanvas`, kept in `ChartAppearance::canvas`), the
painting is `src/ui/chart/canvas.rs` and `ChartCard::body`, the page is
`src/ui/chart/settings/canvas.rs`, and templates are `src/model/chart/template/mod.rs`,
`src/store/chart_template.rs` and `src/ui/chart/settings/templates/mod.rs`.

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
  only when the chart has a pane; with none, the row says so. Panes are not resizable, so the
  line has no drag behaviour to preserve.
- **Crosshair.** Colour, opacity, thickness (1–4 pt) and style. The vertical line runs through the
  price chart and on through every pane; the horizontal line and price tag belong to the chart
  under the pointer. It is drawn above the series, the scripts and the drawings. Restyling does
  not move it. (The crosshair follows the pointer freely; it does not snap to a bar.)
- **OHLC values.** On/off for the box of values in the plot's top-left corner while the pointer is
  over the chart (open, high, low, close, and volume, `Mkt`, constructed-bar and script-candle values
  where they apply). On by default. Off hides the whole box; the crosshair lines, the price and time
  tags and the ring on a line chart's bar stay. The switch is DegenView's own, not stated by the
  TradingView pages cited above. Snapshots never contain the box (it belongs to the crosshair).
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
  hovered while the pointer is over it, so it does not vanish under the pointer. A collapsed or
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

### Panes

Pane order is the order of the scripts on the chart (`PineInstance` list), so moving a pane
reorders that list and there is no second representation. Collapse is stored on the instance
(`pane_collapsed`, saved with the chart; the script still runs). Maximize is view state and is
not saved. Remove is the same removal as Scripts ▸ Remove from Chart. Layout is
`model::chart::canvas::split_panes`: expanded panes share 30 % of the height, a collapsed one is a 24 pt
strip, and a maximized pane takes everything but the strips and the time axis; the price chart
then draws nothing but its time axis. Built-in indicators (volume, RSI, Bollinger) are drawn
inside the price plot, not in panes, so they have no pane buttons.

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
- Not visually compared against TradingView side by side (no reference available offline); the
  rows above say which rules are documented and which are inferred.
