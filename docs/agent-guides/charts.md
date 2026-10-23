# Chart guidance

Use [chart types](../chart-types.md), [chart canvas](../chart-canvas.md),
[drawing tools](../drawing-tools.md), and [portfolio cards](../portfolio-cards.md) for detailed
behavior.

## Rendering and prices

- `ui::chart_card::ChartCard` owns the painter loop. `ui::series_render` draws every main series;
  `model::chart_look` decides bar appearance. Do not add type-specific drawing to `chart_card.rs`.
- Chart state lives in `workspace::Chart`. Draw grid, main series, overlays/indicators, then labels.
  The price transform supports linear and log scales.
- Every chart price uses `ChartAppearance::price_format()` and `PriceFormat`, never
  `format::price`. This includes axes, tags, crosshair/OHLC, ruler/Fibonacci, and high/low markers.
- Automatic precision is deliberately not the Swift app's: no decimals from 1,000, two from 1,
  then two significant digits below 1 (`0.12`, `0.056`, `0.0031`); it counts leading zeros below
  `0.001` (`0.0₅278`). Axis ticks still add the places their spacing needs. It also drives the
  watchlist, which uses `PriceFormat::AUTO`.
  `axis_decimals` (0–12) and `axis_small_prices` stay chart-local across template application.
  Grid ticks use `format_tick`; `ui::price_text` paints the Unicode subscript count. Other screens
  may use `format::price`; watchlist prices use `PriceFormat::AUTO`.

## Navigation

- `model::viewport::ChartViewport` is the only chart view state (bars, fractional offset from the
  newest bar, manual price range). Gestures live in `ui/chart_card/navigation.rs` and edit it;
  never keep another offset or range in a painter, the drawing layer or Pine rendering.
- The window is continuous, so bars at the plot's edge may be cut: clip what you draw to the plot
  and place bars with `Window::lead`, never an integer count of blank slots.
- A pan must not rewrite drawings, emit `DrawingCommand`s, rerun scripts (only a script reading
  `chart.*_visible_bar_time` depends on the range) or write the Canvas. Viewport state is
  chart/layout state and is not part of a template.
- See [chart navigation](../chart-navigation.md) for gestures, history paging and what is unverified.
- Linked charts ([design](../chart-sync.md)) translate a viewport to a time range and back; they
  never copy an offset, a bar index, or anything vertical. Followers are edited through
  `ChartCard::follow_sync` before they draw, and only the user's own navigation is published, never
  a follower's change or a live bar. The `SyncHub` belongs to one `ChartWorkspace`; do not make it
  global. Price-based chart types do not take part.

## Types, canvas, and templates

- Never modify canonical candles. `model::synthetic` builds price-based types; the cached
  `constructed_series` feeds scale, indicators, Pine, alerts, drawings, and replay. Standard types
  keep real OHLC for scripts.
- Type and settings have one persistence path: `ChartAppearance::style`. New fields need
  `#[serde(default)]`. Cosmetic restyling must not change `synthetic::Construction`.
- Builders are pure and prefix-preserving because replay depends on it. Add a fixture for changed
  construction rules and document anything unverified against TradingView.
- Pine's `plotcandle`, `plotbar`, and `barcolor` remain independent layers ordered by
  `PineInstance::visual_order`.
- `ChartAppearance::canvas` is presentation-only. Canvas changes must not alter candles, rebuild
  constructed bars, or rerun indicators/Pine. New colors are `Option<Rgba>` so `None` follows the
  theme.
- Resolve canvas/theme once per frame with `CanvasStyle::resolve`; painters must not fetch canvas
  colors from `theme::*`. Grid and axis labels share `canvas::{price_ticks,time_ticks}`. Keep price
  range, visible-window, and pane-split geometry pure in `model/canvas.rs`.
- Pane order and collapse live on `PineInstance`; maximize is card view state. Buttons report
  clicks and `ChartCard` applies them.
- Templates snapshot `ChartAppearance`. Apply with `ChartCard::apply_template`, and plan all
  apply-to-all changes before applying any, so type changes pass through `set_chart_type`.
  `ChartTemplateStore` re-reads before writes and refuses to overwrite undecodable rows.

## Drawings

- A drawing is pinned to time+price. Fibonacci, the brush and the ruler have variants of their
  own; every other tool, the trend line included, is
  `Drawing::Shape { geometry: Geometry, style, text, locked, hidden, visibility }`. New fields
  need `#[serde(default)]`. Nothing stores a pixel or a derived corner. The app has not shipped
  an older stored form that must keep reading, so do not add compatibility readers.
- A style field only some tools use is left out of the stored row while it holds its default
  (`skip_serializing_if`): `DrawingStyle` is written whole, and every drawing and every synced
  record would otherwise carry the levels of a pitchfork.
- A tool is one row of `Tool::spec()` (`Placement`, `Caps`) and `Tool::default_style()`. Add a
  tool there, a `Geometry` variant, its projection (`shape::shape`, or `line_shapes::project` for
  the lines, channels and pitchforks) and an icon; the controller, toolbar, dialog and tests
  (`every_tool_…` loops over `Tool::GROUPED`) pick it up. Do not add `match tool` to pointer
  handling. The two flyouts are `ToolGroup::MENU` and `ToolGroup::LINE_MENU`; Fibonacci is in
  neither.
- A tool whose anchor means only part of itself (a level's time, a regression's price) says so in
  `Geometry::reads` and restores what an edit may not change in `Geometry::constrain`.
- Geometry lives in `ui/drawing_geometry.rs` and `ui/line_geometry.rs` as pure functions on `Pos2`
  with their own tests. Polygons fill by the even-odd rule; translucent freehand strokes are
  painted from a coverage map (`rasterize_stroke`), never overlapping triangles. A line with no end
  is cut to the plot with `line_in_rect`; never draw from an enormous coordinate.
- A drawing's text is part of it: `Drawing::set_text`, the inline editor in
  `drawing_layer/text_edit.rs`, and `TextPlace` for where the painter puts it. Axis tags are painted by `paint_frame`'s label pass, never by the drawing
  painter (it is clipped to the plot).
- The layer never mutates drawings: it returns `DrawingCommand`s in the `DrawingFrame`, and
  `drawings::apply_commands` applies them. One batch is one undo step; use `Style` for restyles
  (merged in bursts), never one command per pointer move. Previews and cancelled drafts are not
  commands.
- `PersistedChart.drawings` decodes leniently: a row this version cannot read stays as
  `Drawing::Unreadable` and is written back untouched. Skip it in anything that draws, picks or
  edits (the Object tree lists it, deletable only, by `Drawing::stored_id`).
- **Older builds meet your changes through iCloud.** A new tool is a new `Geometry` variant and needs
  nothing more. A new field on an existing tool that an older build would silently drop on re-save
  needs `"since": N` in the drawing body and `DRAWING_FORMAT` raised to N (`model/drawings.rs`); without
  it the older build strips the field and syncs the stripped copy over yours. The full table is in
  [iCloud Sync](../icloud-sync.md#newer-versions).
- Drawings are main-pane only and use the bars the chart draws (constructed and replay-cut); never
  index candles directly.
- The text, note and content tools ([Annotation tools](../annotation-tools.md)) add rules to the
  above. A box is sized with `Projection.measure` (the context that paints), never a guess. A shape with
  nothing painted where it is to be clicked needs `Screen.hit`. A tool that moves with the screen
  (a pinned table) has no anchors and moves through `edit::translate`. Content is named, never carried:
  a drawing holds a picture's hash or a link, and a new picture goes through
  `service::image_ingest` on a worker, never the UI thread. Nothing contacts a service except the
  Preview button on a Post card, after a one-time allow.
- `Drawing::sample` and `sample_content` (tests) make a drawing of any tool; the every-tool loops must
  keep covering the content tools rather than skip them.

## Context menus

- A right-click is answered by one menu: a drawing's (`ui::drawing_layer::menu`), else the chart's
  (`model::chart_context` builds the entries, `ui::chart_context_menu` draws them,
  `workspace/context.rs` runs them). Add an entry as a `Command`, a row in the model's `menu`, and
  an arm in `run_context_command`; the menu never mutates anything itself.
- The click is captured once as a `ChartContext` (chart, market, time, price, target) through the
  card's `Projection` and `PriceScale`. Never recompute a menu price from the live pointer, never
  add another price-to-pixel formula, and always `chart_context::check` the snapshot before
  acting.
- Opening a menu, an order entry or Add alert never places an order or saves an alert: they open
  the paper ticket and the alert editor for the user to review.
- Drags and clicks that move the chart or draw are primary-button only.
- An alert's tag is draggable to a new level. The card only reports an `AlertMove`; the workspace
  turns it into `WorkspaceAction::MoveAlert` (`workspace/context.rs`), so the card never edits a
  `PriceAlert`, and a percentage alert or a replay is never draggable.
- Cursor time lock, the Table view feed and the alert lines are card state that is never saved and
  never part of a template or linked navigation. Table and Object Tree read the bars and state the
  chart already draws; the tree is a view over the drawings, indicators and scripts, not a store.
- Undo is one typed stack (`Snapshot`): drawings, or indicators and scripts. Bulk changes are one
  `apply_drawing_commands` batch or one `remember_indicators` entry. See
  [chart context menus](../chart-context-menu.md).

## Snapshots

- A snapshot is the live frame drawn by `ChartCard::paint_snapshot` through the same
  `frame_layout`/`paint_frame` as `body_inner` (`ui/chart_card/frame.rs`). Never add a second chart
  painter; add to `paint_frame`, or to the `Overlays` hook if only the live chart has it (anything
  a pointer can reach). `snapshot_tests.rs` compares both frames shape for shape and will fail if
  they drift.
- `paint_snapshot` is `&self` and read-only: no Pine run, no cache write, no viewport, pane or
  replay change. Keep new chart state it reads cloneable or `Arc`.
- The frame is frozen on the UI thread in one pass; saving, rasterizing, PNG and upload run on
  `work_queue::SNAPSHOTS` and hand clipboard/browser work back as an `Outcome`. Never report a copy,
  save or open before it has happened.
- Trading overlays are off by default (`snapshot.prefs`); uploads need a configured host and the
  user's consent per host. See [chart snapshots](../snapshots.md).

## Portfolio cards

- A portfolio card is a `workspace::Chart` with `card: Some(...)`; its symbol is a key and source is
  an unread placeholder. `portfolio_card::CardKind` is not Pine's `ChartKind`. Every market-data
  loop must skip non-market cards via `Chart::is_market()`: fetching, live feeds, paper,
  scripts/alerts, replay, MCP, apply-to-all, and layout previews. `Chart::fetch` is a no-op for cards.
- `PortfolioCardEnv::view` alone reads ledgers, quotes, and history. Painters accept `CardData` and
  never access hosts/network. Each `CardRuntime` owns and drops a sibling quote feed demand.
- Favorites use `WatchlistInstrument.card` and `PortfolioCard::instrument_id()`. The card field,
  not provider, controls display/open/export behavior. Opening focuses or adds the card; it never
  replaces a market in the focused chart.
- A card's `portfolio_id` and the key derived from it are translated to shared ids when a layout or
  watchlist syncs (`sync::ids::map_card`); a new place that stores a portfolio id must be added to
  that translation.
- Top Performers ranks each market's candles without FX and scales today's value by return,
  assuming unchanged quantity.
