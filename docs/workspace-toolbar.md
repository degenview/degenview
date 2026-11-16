# Workspace toolbar

The flat bar is shared by every tab kind. `src/ui/kit/bar/` is the engine: `Button`, `Item`, the
measuring, `arrange`, painting and `show`, generic over a click action `A` and a popup key `M` (a
button with `with_menu(key)` gets that key handed to the caller's `popup` callback once it is
drawn, only while enabled). `kit::bar::HEIGHT` (44) is the height of every tab's bar, on
`theme::chrome()`; the chart's toolbar, the Portfolio's (`ui/portfolio/tab/bar.rs`, see
[Portfolio page](portfolio-page.md)) and the Pine Studio's (`pine_studio/bar.rs`) differ only in
their items, so switching tabs never changes the window's chrome. Button ids are scoped by the
panel's `ui.id()`. Do not draw a tab's top controls with anything else.

**Pine Studio bar.** Left: script list toggle, preview-chart position menu, preview toggle |
market button (name, then the last price and change as quiet text; opens the market picker,
`preview::market_picker`) | timeframe dropdown (the same [picker](#timeframe-picker) as the chart
toolbar's; its warning line looks at the preview chart's source). The market and timeframe act on the
preview chart and are disabled with no open script or a hidden chart. Right: assistant, profiler
(tinted when it needs attention) and debugger, for an open script. The market and timeframe used to
sit in a 52 pt strip over the preview chart; that strip is gone.

**View menu (macOS).** Between Edit and Window: Pine Studio and Portfolio
(`NativeCommand::OpenPineStudio` / `OpenPortfolio`), each opening its one tab or showing the open
one, like the chart toolbar's two buttons, which stay. Windows and Linux have no menu bar; the
buttons are their entry.

`src/ui/workspace/toolbar.rs` draws the bar above a tab's charts; `src/workspace/toolbar.rs` builds its
input and applies what it asks for. It is flat like TradingView's and LuxAlgo's: no capsules, a
faint tint on hover, hairline separators between groups, small corner radius.

## Layout

Left to right: symbol (the focused chart's market, opens the [symbol picker](symbol-picker.md)) and a
⊕ beside it that compares another market (the same picker in Compare mode as the chart settings'
Compare page, `open_compare_picker`; disabled where a chart cannot take one) |
timeframe dropdown (one button, the current timeframe and a chevron that
turns up while the picker is open) | Indicators | chart type | Replay, Alert | Undo, Redo. Right end, from the edge inwards: watchlist toggle, Pine Studio, Portfolio |
snapshot menu, chart settings | layout menu.

- Every button is a `Button` (id, glyph, label, tooltip, enabled, lit, action). `measure` lays the
  labels out, `arrange` places the two runs, `paint_button` draws one. Tests use `arrange`, so the
  geometry they check is the one drawn.
- Width: when both runs with labels do not fit, the Indicators label drops and the
  icons stay (Replay and Alert are icon only at every width, their tooltips name them); a button without an icon (the timeframe, the layout name) keeps its text. If the runs still
  collide, the right run drops buttons from its left end (the layout menu first). Never overlap. The
  narrowest window is 960 wide and keeps everything but possibly the layout name (the icon-only
  bar with the layout menu measures 975 points, 910 before the separators and the compare button; at 960 the layout menu is the one that drops, and every other button is tested there).
- The colours are neutral (`kit::primary` tints, `text_primary` / `text_secondary` /
  `text_tertiary`); the only colour is Replay's orange while a replay is open (state, not a control).
- `replay/bar.rs` (the strip under the toolbar while a replay is open) uses the same flat style: its
  `flat_tint` is the toolbar's hover/pressed tint at the same radius. Its one solid control, the
  start row's main action, is still inked like the label colour.

## Timeframe picker

`ui/kit/timeframe_picker/`, shared by this bar and Pine Studio's: the button opens a popover under it
(`kit::menu_frame`, as wide as four chips: chip width comes from the widest label or what the footer
hints need, never a literal) and reports a `TimeframePick`; the caller applies it. The timeframe stays
one per tab: `Chart` is `ToolbarAction::SetTimeframe` (`reload_for_timeframe`, then Tab Linking's
`announce_timeframe` when it changed).

- **Field** (`kit::search_box` with the clock icon): read live by `Timeframe::parse`, so `22m`, `3h`,
  `2D`, `1W`, `3M`, `1Y`, and a bare `22` (minutes). Seconds are refused ("Seconds aren't supported");
  any refusal shows its message in one caption line and Return does nothing. A valid timeframe no chip
  shows gets a row above the sections, "Chart 22m" with its description. An empty field points at the
  current timeframe.
- **Chips** by section (`Timeframe::section`): MINUTES `1m 5m 15m 30m`, HOURS `1h 4h 8h 12h`, DAYS AND
  UP `1D 1W 1M 1Y` (`Timeframe::PRESETS`), four to a row (a fifth wraps). `3M` is typed or pinned. Pinned timeframes join their section; each
  section is sorted by duration. The current one is lit and semibold.
- **Keys**: Return charts the highlighted timeframe, ⇧Return charts it and pins it (a preset or an
  already pinned one is not added again), ↑↓ move between rows keeping the column, ←→ walk the chips
  while the field is empty (with text they move the caret), Esc closes. Typing takes the highlight back
  from the arrows. The keys are taken before the field sees them (`keys.rs`).
- **Pinning**: a pinned chip has a right-click menu with Unpin. It is a popup of its own
  (`unpin.rs`), because egui keeps one memory popup open and `fitted_context_menu` would replace the
  picker; while it is open the picker ignores outside clicks, so the first click away only dismisses it.
- **Warning line**: when the highlighted timeframe is one a chart's source cannot draw
  (`DataSource::serves`), "CoinGecko can't chart 1m" (two sources named, more counted). The sources are
  the tab's market charts (overlays are not looked at) or the preview chart's. It warns; it does not
  stop the pick, and the chart then shows why it cannot load.
- **Where pins are stored**: the `timeframePicker.pinned` setting, a JSON list of labels
  (`store::pinned_timeframes`), read through `Store::pinned_timeframes_cached` (`frame_cache`, so no
  parse per frame), one list for every tab and Pine Studio. Portable, last writer wins
  (`sync::record::policy`). A label that does not parse is dropped on read.
- The footer is `kit::hints::hint_row`, also the symbol picker's; the field is `kit::search_box`.
  `Icon::Shift` draws ⇧ because its glyph is missing from some Linux fonts.
- `gallery_of_the_timeframe_picker` (in the toolbar tests; ignored, set `DEGENVIEW_GALLERY_DIR`) writes
  the picker in both themes: closed, open, typed, error, pinned, warning.

## What acts on what

- **Symbol** shows the focused market chart's name as its exchange writes it and never drops its label
  (`keep_label`) when the bar narrows; it is enabled with no chart (a pick then adds one). ⌘K / Ctrl+K
  does the same.
- **Chart type, Indicators, chart settings** act on the focused market chart
  (`ChartWorkspace::focused_market_chart`). They are disabled with none (empty tab, a portfolio card
  focused), the chart type for a line-only source, settings for a CoinMarketCap gauge, Indicators for a
  prediction market or a gauge (`DataSource::takes_indicators`). A card header has
  no chart-type button and no "+" any more; a card's title still opens its settings.
- **Replay** moved here from the tool rail (`ChartWorkspace::toggle_replay`); enabled with a market
  chart (CoinMarketCap excepted) or while a replay is open.
- **Undo / Redo** are the workspace's undo stacks (`undo.rs`), the same as the keyboard shortcut;
  disabled when the stack is empty. They also take back a replaced chart market.
- **Alert** starts a price alert on the focused chart's market (`create_alert_for_focused_chart`); it
  opens the alert editor, never saves one, and is disabled for a market with no price feed.
- **Snapshot** (camera, right of chart settings) opens the snapshot menu for the focused chart
  (`snapshot_menu::menu`). Both were buttons in each card's header, which now has neither. Its
  rows: Download image, Copy image, Select portion (a mode of the workspace, see
  [Snapshots](snapshots.md#select-portion); a selection opens the [Studio](snapshots.md#the-studio-window)), the hosted rows, then Configure Snapshots….
- The chart-type pick goes through `set_chart_type_for_mcp`, so the toolbar, the MCP tool and the
  menu share one rule.

## Verified and not

Unit tests cover layout at several widths (the timeframe button is the same width at all of them), that every new button asks for its action and that a
disabled one never does, and the workspace methods. `gallery_of_the_toolbar` (ignored, set
`DEGENVIEW_GALLERY_DIR`) writes pictures of the bar in each state. Not checked in a real window on
Linux or Windows.
