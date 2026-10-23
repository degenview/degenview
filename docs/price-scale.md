# Price scale modes

A chart reads its price axis in one of four modes, chosen per chart: **Regular**, **Percent**,
**Indexed to 100** and **Logarithmic**. They are mutually exclusive (`PriceScaleMode`, one enum, so
"percent and log" cannot be written) and replace the old Log on/off switch and its toolbar button.

The mode is chart/layout state, like the viewport: not part of `ChartAppearance`, so templates
neither carry nor reset it. It is saved with the chart in the session and in saved layouts, is part
of the layout fingerprint (changing it marks a layout unsaved and starts autosave), and syncs with
the layout record. Each chart has its own, in a tab, a window or a duplicate.

## What each mode means

| Mode | Axis reads | Equal... take equal heights |
|---|---|---|
| Regular | the price | price differences |
| Percent | `100 * (price - ref) / abs(ref)`, signed (`+10%`, `0%`, `-25%`) | price differences |
| Indexed to 100 | `100 + 100 * (price - ref) / abs(ref)` (`110`, `100`, `90`) | price differences |
| Logarithmic | the price | price ratios |

**Percent and Indexed are linear in the price.** They move no pixel: candles, wicks, indicators,
drawings, alerts and orders sit exactly where they do in Regular. Only what the axis says changes
(ticks, the tags on the axis, the crosshair pill). That is also why switching modes cannot change a
stored price: drawing anchors, alert levels, order and stop prices and Pine inputs stay real prices,
and a click on the axis or the plot (right-click menu, Add alert, order entry, a drag) goes through
`PriceScale::price`, which is the inverse of the map and never of what the axis prints. Percent and
Indexed never reach an order as `10` or `110`.

`abs(ref)` keeps the sign right for a negative reference (a spread, a PnL series).

## The reference price

The reference is the **close of the first bar the plot shows**: `bars[window.start].close` of the
series the chart draws (the constructed bars of Renko, Kagi and the other price-based types; the
replay-cut candles during a replay). It is resolved inside `ChartCard::view()`, the one function
every part of a frame asks for the scale, so the grid, the labels, the tags and the axis width
agree. Panning or zooming the time axis moves it, and the axis follows: `0%` is always the bar at
the left edge. The map does not move with it.

A reference that is not a finite non-zero price (no bars, a close of zero) is **unavailable**: the
axis reads prices, as Regular, and comes back by itself when there is a bar to measure from. Nothing
divides by zero and nothing falls back to an arbitrary price.

Sources: TradingView's support page for the Indexed scale says the first value on the chart is the
base set to 100 and later values are 100 plus their percentage change; the lightweight-charts
percentage and indexed modes are the same linear transforms of the first visible value.
**Inferred, not compared against a live TradingView chart:** that a first bar the plot's edge cuts
still counts as the first, and what the reference does through replay and older-history paging
(here it follows the first bar shown). A comparison-symbol series does not exist in the app, so the
prediction markets' outcome lines (`ChartCard::series`) simply share the main series' reference.

## Axis ticks and labels

`PriceScale::axis_ticks` is the one list the grid lines and the labels come from (the grid and the
axis-width pass used to build it twice). In a relative mode the ticks are picked **in the value the
axis shows**, with the same nice steps as a price axis (1, 2, 2.5, 5 x 10^n), then mapped back to
prices: `-10%`, `0%`, `+10%` are round, `0%` / `100` lie exactly on the reference, and a grid line
never sits away from its label. Logarithmic uses the existing decade ticks.

Labels: Percent carries its sign and never reads `-0%` (zero is `0%`); places follow the tick step
(`+2.5%`) and a tag writes two (`+12.35%`); Indexed is a plain number; thousands are grouped. The
axis is as wide as the widest of the tick and tag labels, measured in the mode's own text.

On the axis column the **tags** read as the axis does: the last-price tag, the constructed-bar tag,
the crosshair pill and a drawing's price tag. **Everything else keeps real prices**: the header and
the OHLC legend, alert tags and paper order/position pills (trading semantics), Fibonacci, ruler,
arrow and price-label text, high/low markers, "Copy price", the order and alert entries, indicator
panes (they have a scale of their own that the mode never touches) and Pine labels.

## Logarithmic

Logarithmic keeps its existing behaviour (margins in log space, drag and zoom by ratio, decade
ticks, regression and Fibonacci in logs). What is new is the rule for a price a logarithm cannot
take. If the visible range holds a price at or below zero (an oscillator crossing zero, a negative
series) the frame is **drawn linear**, consistently: the range, the ticks and the map all agree, no
price is clamped to a tiny positive one, and nothing is NaN. The mode stays Logarithmic, so the log
scale returns when the data allows; the button's tooltip and the row's tooltip say why it is
linear. A saved manual price range that a log scale cannot draw falls back to autoscale
(`PriceRange::is_valid`), as before.

Pasting drawings on a log chart keeps ratios (`DrawingClip::placed`); it refuses a non-positive
price, so in a view that is drawn linear for that reason the paste is refused with its usual
message. Unverified edge.

## Switching, panning, zooming

A switch changes only the map and the labels: no refetch, no Pine run, no change to the horizontal
position, the bars shown or the drawings. A **manual price range** (real prices) is kept when it is
valid for the new map, otherwise the chart autoscales (the precedent set by log scale before). Wheel,
drag and the price-axis drag edit `VerticalViewport` as before, in log space only while the frame is
logarithmic (`Navigation::logarithmic`).

## The corner button

A small button sits where the price axis meets the time axis (`ui::scale_menu`): the cell right of
the plot and under it, centred, 20 pt (smaller in a tight cell, absent when the cell cannot hold it,
the chart has no plot, or the card is a script preview). It lights in the accent colour when the
mode is not Regular. It opens a menu in the shared kit style (`kit::menu_popup`, `menu_scope`,
`menu_row` with the check and shortcut slots): one row checked, Percent `⌥P` and Logarithmic `⌥L`
(`Alt+P` / `Alt+L` elsewhere) in the muted shortcut slot. A pick closes the menu, makes that chart
the focused one and goes to `ChartWorkspace::set_scale_mode`, as do the shortcuts and the price-axis
right-click menu (which lists the four modes in place of the old log toggle): one path, no state of
the menu's own.

The chart settings sheet's **Price axis** section (Appearance tab) has a **Scale** row with the same
four modes. It is not part of `ChartAppearance`, so a pick does not go through the appearance
write-back (which is what templates snapshot): it returns `ChartSettingsAction::SetScaleMode`, and
the workspace sets it through the same `set_scale_mode`. The row, the corner menu and the shortcuts
therefore always agree, and a template or "apply to all" never carries the mode.

While the corner menu is open it owns the keyboard (`ChartCard::body_inner`): the drawing layer is
told its keys are blocked, and `scale_menu::close_on_escape` spends Escape (egui's popup closes on
an Escape without taking it, so it would otherwise also end a draft, disarm a tool or deselect a
drawing behind the menu). `price_scale_tests` drives this through a real card with a tool armed and
a draft in progress. A time label that would hang over the plot's right edge waits for the chart to
move (as price labels do at the plot's top and bottom), so a label never runs into the corner.

Keyboard: nothing is highlighted on open; Down lands on the first row and Up on the last, the arrows
then wrap, Enter or Space picks the highlighted row, Escape or a click outside closes. The pointer
moves the highlight as it moves.

**Shortcuts** (`ui::shortcut`): TradingView's documentation lists Alt+P for percentage and Alt+L for
logarithmic scale, as toggles. Here a press of the active mode's shortcut goes back to Regular. They
act on the focused chart, and leave a focused text field alone (Option+letter types a character on a
Mac). TradingView documents no shortcut for Indexed to 100, so none is invented.

The button is live-only: `paint_snapshot` never draws it, and `snapshot_tests` checks that the
image is the same with or without it. It is painted from `LiveOverlays::finish`, reports through
`OverlayResults` / `ChartCard::take_scale_request`, and the card never edits workspace state.

## Persistence and older builds

`PersistedChart` stores `scale_mode` (a name; an unknown one reads as Regular, so a newer build's
mode never fails a row) **and** still writes `log_scale`, derived from the mode. Reading prefers the
explicit mode; a row with no `scale_mode` (written by v0.4 to v0.6.0-rc.1) takes its mode from
`log_scale`, so no saved log chart loses its setting, and a row with neither opens as Regular.
`log_scale` is kept so that an older build, where it is a required key, can still decode a layout a
newer one saved (and syncs it as before).

Mixed-version caveat (read from the code, not tried between two real builds): an older build that
opens and re-saves a layout drops `scale_mode` and keeps the log switch, so a Percent or Indexed
chart opens as Regular there, and `layout_structure_hash` of that copy differs, which can produce a
"conflict from" copy of the layout instead of a quiet last-writer-wins. There is no format marker
for layout fields (`DRAWING_FORMAT` is for drawings only). `ChartConfig`, the in-memory layout
fingerprint, carries the mode and is never stored.

MCP: nothing reads or writes the mode; `get_candles` and the drawing tools work in real prices.

## Not verified

* Against a live TradingView chart: the reference rule at a cut first bar, replay, and the exact
  look of its menu.
* In a real window on macOS beyond a launch: a release build starts and its toolbar has no Log
  button, but no click was driven into it. The corner button, the menu (click, arrows, Enter,
  Escape, outside click, with a tool armed and a draft in progress) and the modes are exercised by
  headless egui tests that run the card's own `body`, and `gallery_of_the_scale_modes` (an `#[ignore]`d
  test that writes PNGs) shows the four modes and the open menu.
* High-DPI and every theme by eye; layout of the label at extreme percentages (the axis width
  adapts and is capped like a price axis).
