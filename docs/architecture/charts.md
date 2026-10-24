# Charts, drawings, and replay

See [chart types](../chart-types.md), [chart canvas](../chart-canvas.md), and
[portfolio cards](../portfolio-cards.md) for feature rules.

## Canvas and templates

```text
Canvas controls → ChartAppearance.canvas → CanvasStyle::resolve → painters
                                      └→ model::canvas geometry/ticks
Templates → ChartTemplateStore → ChartCard::apply_template
Apply to all → plan_apply_to_all → each chart's ChartCard
```

Canvas state is saved with charts/layouts and included in layout fingerprints. It is presentation
only: it cannot change candles, constructed bars, indicators, or Pine execution. Shared pure
geometry owns ticks, price range, visible window, and pane splits.

A template contains appearance only—type/settings, colors, and canvas. It excludes markets,
indicators, scripts, drawings, and alerts. `ChartCard::apply_template` preserves data and routes type
changes through `set_chart_type`.

## Drawings and undo

`Drawing` holds the brush and the ruler plus one `Shape` variant that
carries a typed `Geometry` for every newer tool, all with time+price anchors. The interactive layer
(`ui::drawing_layer`) is one controller driven by each tool's `Placement`; it reports
`DrawingCommand`s and never mutates drawings. Lines, channels and pitchforks are projected by
`drawing_layer/line_shapes.rs` from the pure `ui::line_geometry` and `model::regression`, and a
line's text is edited in place by `drawing_layer/text_edit.rs`. The chart patterns, Elliott waves
and cycles are projected by `drawing_layer/pattern_shapes.rs` from `model::{pattern, elliott,
cycle}` (pivots, ratios, degrees, repeats) and the pure `ui::pattern_geometry` (apex, arcs, sine);
see [chart patterns, Elliott waves and cycles](../pattern-tools.md). Positions, forecasts, the
bars pattern, ghost feed, sector, anchored VWAP, volume profiles and measurers are projected by
`forecast_shapes.rs` and `volume_shapes.rs` from `model::{position, volume_profile, ghost,
bars_pattern, measure}`; see [forecasting, volume and measurement tools](../forecast-tools.md). The
Fibonacci and Gann tools are projected by `fib_shapes.rs` and `gann_shapes.rs` from `model::{fib, gann,
fib_style}` and the pure `ui::fib_geometry` (sampled curves, annuli, fans); see [Fibonacci and Gann
tools](../fibonacci-gann-tools.md). See
[drawing tools](../drawing-tools.md). Rulers and crosshairs are transient.

`ChartWorkspace` keeps session-only per-workspace undo/redo entries containing one chart id and a
`Snapshot`: its complete pre-command drawing list, or its built-in indicators and scripts before a
removal. Undo restores only that chart. A batch that changes nothing is no step, and restyles in
quick succession merge into one. ⌘Z/⇧⌘Z are egui key events handled in
`ChartWorkspace::handle_keyboard` (there is no native Edit menu) and are skipped while a text field
has the keyboard.

## Context menus

A right-click on a chart is answered by the drawing's own menu or by the chart menu
(`model::chart_context` builds it from a click snapshot and the chart's facts,
`ui::chart_context_menu` draws it, `workspace/context.rs` owns the open menu and runs the chosen
command). The same module hosts Table view (`model::chart_table`, fed by the card from the bars it
draws), the Object Tree (`model::object_tree`), the drawing clipboard (`model::chart_clipboard`),
the cursor time lock, and the alert lines (`model::alert_line`). See
[chart context menus](../chart-context-menu.md).

## Replay

`ReplayEngine` owns the workspace timestamp, playback step, and status. Charts retain immutable
canonical history and expose a binary-searched prefix through `displayed_candles`; persistence never
stores timeline position. The replay bar controls selection, transport, seek, speed, and interval.
Esc/Return to live cancel selection. Until granular replay fetching exists, each interval replays its
own bars.
