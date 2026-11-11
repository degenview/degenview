# Chart guidance

Details: [chart types](../chart-types.md), [chart canvas](../chart-canvas.md),
[drawing tools](../drawing-tools.md), [portfolio cards](../portfolio-cards.md).

## Rendering and prices

- `ui::chart_card::ChartCard` owns the painter loop, `ui::series_render` draws every main series,
  `model::chart::look` decides bar appearance. No type-specific drawing in `chart_card.rs`.
- State: `workspace::Chart`. Order: grid, main series, overlays/indicators, labels.
- `ui::chart_card::PriceScale` is the only price<->y map (linear, or log while the frame can draw
  one), carrying `PriceScaleMode` and reference. Percent and Indexed to 100 change only what axis,
  tags and crosshair say, never a stored price. Read [price scale modes](../price-scale.md) before
  touching axis labels, ticks, tags or the corner button. Axis-column tags: `PriceScale::tag_label`,
  not bare `PriceFormat`.
- Every chart price (axes, tags, crosshair/OHLC, ruler/Fibonacci, high/low markers):
  `ChartAppearance::price_format()` and `PriceFormat`, never `format::price` (fine on other
  screens). Watchlist uses `PriceFormat::AUTO`.
- Automatic precision deliberately differs from Swift: no decimals from 1,000, two from 1, two
  significant digits below 1 (`0.12`, `0.056`, `0.0031`), leading-zero count below `0.001`
  (`0.0₅278`). Axis ticks add places their spacing needs. Grid ticks: `format_tick`; `ui::price_text`
  paints the subscript count.
- `axis_decimals` (0–12) and `axis_small_prices` stay chart-local across template application.

## The header over the plot

- Plot reaches the card's top edge; header (logo, symbol, compare pills, indicators, price) is drawn
  over it after the body in `workspace/grid.rs`, via `ChartCard::set_top_inset(HEADER_HEIGHT)`.
  `PriceScale.top`/`height` start below it (`view`'s `top_inset`): bars clear it by default, may be
  panned under. Plot, grid, clip and pointer areas are the whole card.
- Top-anchored items (OHLC/compare readouts, replay pick tag and badge, logs-range tag,
  paper-trading pills) use `ChartCard::below_header(plot)` or `top_inset`, never `plot.top()`.
  Vertical pan converts pixels via `scale.height`, not `plot.height()`.
- Snapshot: no header, zero inset, `snapshot_size()` drops the strip; geometry is the plot below the
  header.
- No interaction on the header's empty space: no widget there, a drag pans. Portfolio cards keep
  header-above-body.

## Navigation

- `model::chart::viewport::ChartViewport` is the only view state (bars, fractional offset from
  newest bar, manual price range), edited by gestures in `ui/chart_card/navigation.rs`. Never keep
  another offset or range in a painter, the drawing layer or Pine rendering.
- Continuous window, edge bars may be cut: clip to the plot, place bars with `Window::lead`, never
  an integer count of blank slots.
- A pan never rewrites drawings, emits `DrawingCommand`s, reruns scripts (only
  `chart.*_visible_bar_time` readers depend on range) or writes the Canvas. Viewport is
  chart/layout state, not template.
- Gestures, history paging, unverified items: [chart navigation](../chart-navigation.md).
- Zoom In is a workspace mode, not a `Tool`: transient `ChartCard` rectangle, maths in
  `model::chart::viewport::zoom`, edits the same `ChartViewport` in-frame so linked charts follow.
  No second viewport; never store the rectangle as a drawing
  ([chart navigation](../chart-navigation.md#zoom-in)).
- Linked charts ([design](../chart-sync.md)) map viewport to time range and back; never copy an
  offset, bar index, or anything vertical. Followers are edited via `ChartCard::follow_sync` before
  drawing; publish only the user's own navigation, never a follower's change or a live bar.
  `SyncHub` belongs to one `ChartWorkspace`, never global. Price-based chart types excluded.

## Compared markets (overlays)

Read [chart overlays](../chart-overlays.md) first. Short rules:

- Config: `Chart::overlays` (`OverlayList`); data: workspace's `OverlayHub`. Card gets both via
  `ChartCard::set_overlays`, fetches nothing. Scripts, alerts, drawings, orders, MCP tools read only
  `Chart::symbol`/`source`/`candles`; a test pins it.
- Candles map to chart bars via `model::overlay::align::align` (span midpoint, bars in view only),
  never by index or open time. Missing bar is `None`: no forward fill.
- Rebasing/fitting: `model::overlay::scale`; painters never repeat it. Lines painted in
  `paint_frame` (so snapshots get them), not in `Overlays`.
- Edits: `OverlayCommand`s via `apply_overlay_command` (one undo step, none if unchanged).
- No header-strip chips: controls are the overlay card's (`chart_card/overlay_card.rs`), settings
  `overlay_settings::body`, shared by Compare page and popover. Edit that one body, not copies.
- Nothing caps the count; bound the *work* (in-flight requests, refresh rate, bars in view), never
  the list.

## Types, canvas, and templates

- Never modify canonical candles. `model::chart::synthetic` builds price-based types; cached
  `constructed_series` feeds scale, indicators, Pine, alerts, drawings, replay. Standard types keep
  real OHLC for scripts.
- Type and settings persist only via `ChartAppearance::style`; new fields need `#[serde(default)]`.
  Cosmetic restyling must not change `synthetic::Construction`.
- Builders are pure and prefix-preserving (replay depends on it). Changed construction rules need a
  fixture; document anything unverified against TradingView.
- Pine's `plotcandle`, `plotbar`, `barcolor` stay independent layers ordered by
  `PineInstance::visual_order`.
- `ChartAppearance::canvas` is presentation-only: never alters candles, rebuilds constructed bars,
  or reruns indicators/Pine. New colors: `Option<Rgba>` (`None` follows theme).
- Resolve canvas/theme once per frame with `CanvasStyle::resolve`; painters never read canvas colors
  from `theme::*`. Grid and axis labels share `canvas::{price_ticks,time_ticks}`. Price range,
  visible-window, pane-split geometry stay pure in `model/chart/canvas/mod.rs`.
- Pane order/collapse live on `PineInstance`; maximize is card view state. Buttons report clicks,
  `ChartCard` applies them.
- Templates snapshot `ChartAppearance`. Apply with `ChartCard::apply_template`; plan all
  apply-to-all changes before applying any, so type changes go through `set_chart_type`.
  `ChartTemplateStore` re-reads before writes, refuses to overwrite undecodable rows.

## Drawings

- Pinned to time+price. Brush and ruler have own variants; every other tool (trend line, Fibonacci
  included) is `Drawing::Shape { geometry: Geometry, style, text, locked, hidden, visibility }`. New
  fields need `#[serde(default)]`. Never store a pixel or derived corner. No older stored form has
  shipped: no compatibility readers.
- Style fields only some tools use are omitted at default (`skip_serializing_if`): `DrawingStyle` is
  written whole, else every drawing and synced record carries a pitchfork's levels.
- A tool = one row of `Tool::spec()` (`Placement`, `Caps`) and `Tool::default_style()`, a `Geometry`
  variant, a projection (`shape::shape`; `line_shapes::project` for lines/channels/pitchforks;
  `pattern_shapes::project` for patterns/Elliott waves/cycles) and an icon. Controller, toolbar,
  dialog and tests (`every_tool_…` loops over `Tool::GROUPED`) pick it up. No `match tool` in
  pointer handling. Flyouts: `ToolGroup::MENU`, `LINE_MENU`, `NOTE_MENU`, `PATTERN_MENU`,
  `FORECAST_MENU`, `FIB_MENU`. Over four anchors needs the fixtures' seven: every-tool loops slice a
  seven-point array by `Placement::Points(n)`.
- Fibonacci/Gann (`fib_shapes`, `gann_shapes`; maths `model::{fib, gann, fib_style}`,
  `ui::fib_geometry`): levels in `DrawingStyle.fib: Option<FibStyle>` (`None` = tool defaults, so
  projections read `fib_style(tool, style)`), fixed-size lists for the one level editor. Levels from
  stored prices and bar indexes. Curves sampled only where they cross the plot, hard budget
  (`fib_geometry::sample`); fills are convex pieces cut to the plot (`Screen.backdrop`: seen, never
  picked). Gann angle = price/time ratio from the second anchor, never a fixed screen angle.
  Unhonoured controls are absent from `Features`, so not offered.
  [Fibonacci and Gann tools](../fibonacci-gann-tools.md).
- Names and ratios are `Screen.labels` (`PivotLabel`), laid out by `draw::draw_labels` at paint, not
  projection text. Ratios from stored prices (`model::pattern`), never pixels, so scale-independent.
  Cycles counted in bars (`model::pattern::cycle`, `TimeIndex::index_of`); curves sampled only over
  the plot (`ui::pattern_geometry`). [Chart patterns, Elliott waves and cycles](../pattern-tools.md).
- Forecast/volume/measure (`forecast_shapes`, `volume_shapes`; maths
  `model::{position, volume_profile, ghost, bars_pattern, measure}`) add `Screen.boxes`, `candles`,
  `shade_meshes`. Read only the chart's drawn bars; never fetch or reach an order, alert or script.
  Positions are hypothetical (a test pins that trading and alert engines don't mention drawings).
  `ProjectionKey` includes newest bar's volume.
  [Forecasting, volume and measurement tools](../forecast-tools.md).
- Anchor meaning only part of itself (a level's time, a regression's price): declare in
  `Geometry::reads`, restore what edits may not change in `Geometry::constrain`.
- Geometry: pure `Pos2` functions with own tests in `ui/drawing_geometry.rs`, `ui/line_geometry.rs`.
  Polygons fill even-odd; translucent freehand strokes paint from a coverage map
  (`rasterize_stroke`), never overlapping triangles. Endless lines cut to the plot with
  `line_in_rect`; never draw from an enormous coordinate.
- Text belongs to the drawing: `Drawing::set_text`, inline editor `drawing_layer/text_edit.rs`,
  placement `TextPlace`. Axis tags painted by `paint_frame`'s label pass, never the drawing painter
  (clipped to plot).
- Layer never mutates drawings: returns `DrawingCommand`s in the `DrawingFrame`, applied by
  `drawings::apply_commands`. One batch = one undo step; restyles use `Style` (merged in bursts),
  never one command per pointer move. Previews and cancelled drafts aren't commands.
- `PersistedChart.drawings` decodes leniently: unreadable rows stay `Drawing::Unreadable`, written
  back untouched. Skip them when drawing, picking, editing (Object tree lists them, deletable only,
  by `Drawing::stored_id`).
- **Older builds meet your changes through iCloud.** New tool (new `Geometry` variant): nothing
  more. New field on an existing tool that an older build would silently drop on re-save: add
  `"since": N` in the drawing body and raise `DRAWING_FORMAT` to N (`model/drawings/mod.rs`), or the
  older build strips it and syncs the stripped copy over yours. Table:
  [iCloud Sync](../icloud-sync.md#newer-versions).
- Main pane only; use the chart's drawn bars (constructed, replay-cut), never index candles directly.
- Text/note/content tools ([Annotation tools](../annotation-tools.md)) add: size boxes with
  `Projection.measure` (painting context), never guess. Nothing painted at the click spot → needs
  `Screen.hit`. Screen-fixed tools (pinned table) have no anchors, move via `edit::translate`.
  Content is named, never carried: a picture's hash or a link; new pictures go through
  `service::image_ingest` on a worker, never the UI thread. Only a Post card's Preview button
  contacts a service, after a one-time allow.
- `Drawing::sample`, `sample_content` (tests) build any tool; every-tool loops must cover content
  tools, not skip them.

## Context menus

- One menu per right-click: drawing's (`ui::drawing_layer::menu`), else chart's
  (`model::chart::context` builds, `ui::chart_context_menu` draws, `workspace/context.rs` runs).
  New entry: a `Command`, a row in the model's `menu`, an arm in `run_context_command`. Menus never
  mutate.
- Click captured once as `ChartContext` (chart, market, time, price, target) via the card's
  `Projection` and `PriceScale`. Never recompute a menu price from the live pointer or add another
  price-to-pixel formula; always `chart_context::check` the snapshot before acting.
- Menus, order entry and Add alert never place orders or save alerts: they open the paper ticket or
  alert editor for review.
- Chart-moving and drawing drags/clicks are primary-button only.
- Alert tags drag to a new level: the card only reports `AlertMove`, the workspace makes
  `WorkspaceAction::MoveAlert` (`workspace/context.rs`); the card never edits a `PriceAlert`.
  Percentage alerts and replays never drag.
- Cursor time lock, Table view feed, alert lines: card state, never saved, never in a template or
  linked navigation. Table and Object Tree read what the chart already draws; the tree is a view
  over drawings, indicators, scripts, not a store.
- Pine logs pane: also a view, reads `ChartCard::pine_logs` (run's messages, never saved or
  templated), dropped with its chart. Only level and search options persist (local
  `pineLogs.prefs`).
- Saved scripts reach a chart two ways, both via `ui::script_catalog` (saved indicators/strategies
  by name, re-read at most every 2 s, only while a menu or the Scripts page is open; never call
  `ScriptStore::all_scripts` per frame):
  - Toolbar Indicators picker (card header has no "+") reports `Action::AddScript`; workspace
    applies it with `add_pine_instance_to`, one undo step.
  - Chart settings Scripts page: a switch per saved script. First switch-on adds an instance; after
    that the switch is only `PineInstance::visible`, so inputs, style, order survive switching off.
    Removal is the row's separate x. No workspace handle, so edits aren't undoable.
- Session statistics (`model::session_stats`, `ui::session_stats_view`): read the market's own held
  candles cut at the replay position, never a constructed type's bars; answer once per change of
  bars or question (cached, never per frame). Panel state is card state, unsaved, in no template.
  Every answer carries sample size and a 95% interval and is a historical frequency, never a
  forecast. [Session statistics](../session-stats.md).
- Undo: one typed stack (`Snapshot`) of drawings, or indicators and scripts. Bulk changes = one
  `apply_drawing_commands` batch or one `remember_indicators` entry.
  [Chart context menus](../chart-context-menu.md).

## Snapshots

- A snapshot is the live frame: `ChartCard::paint_snapshot` uses the same
  `frame_layout`/`paint_frame` as `body_inner` (`ui/chart_card/frame.rs`). Never a second chart
  painter: add to `paint_frame`, or the `Overlays` hook if live-only (anything pointer-reachable).
  `snapshot_tests.rs` compares both frames shape for shape and fails on drift.
- `paint_snapshot` is `&self`, read-only: no Pine run, cache write, viewport/pane/replay change. New
  chart state it reads must be cloneable or `Arc`.
- Frame frozen on the UI thread in one pass; saving, rasterizing, PNG, upload run on
  `work_queue::SNAPSHOTS`, handing clipboard/browser work back as an `Outcome`. Never report a copy,
  save or open before it happened.
- Trading overlays off by default (`snapshot.prefs`). Uploads need a snapshot provider on and set
  up, plus user consent per destination. [Chart snapshots](../snapshots.md).

## Portfolio cards

- A `workspace::Chart` with `card: Some(...)`; symbol is a key, source an unread placeholder.
  `portfolio_card::CardKind` ≠ Pine's `ChartKind`. Every market-data loop skips non-market cards
  via `Chart::is_market()`: fetching, live feeds, paper, scripts/alerts, replay, MCP, apply-to-all,
  layout previews. `Chart::fetch` is a no-op for cards.
- Only `PortfolioCardEnv::view` reads ledgers, quotes, history. Painters take `CardData`, never
  access hosts/network. Each `CardRuntime` owns and drops a sibling quote feed demand.
- Favorites: `WatchlistInstrument.card`, `PortfolioCard::instrument_id()`. The card field, not
  provider, controls display/open/export. Opening focuses or adds the card, never replaces the
  focused chart's market.
- `portfolio_id` and its derived key are translated to shared ids on layout/watchlist sync
  (`sync::ids::map_card`); any new place storing a portfolio id must join that translation.
- Top Performers ranks each market's candles without FX, scales today's value by return, assuming
  unchanged quantity.
