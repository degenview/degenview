# Workspace toolbar

`src/ui/workspace_toolbar.rs` draws the bar above a tab's charts; `src/workspace/toolbar.rs` builds its
input and applies what it asks for. It is flat like TradingView's and LuxAlgo's: no capsules, a
faint tint on hover, hairline separators between groups, small corner radius.

## Layout

Left to right: symbol (the focused chart's market, opens the [symbol picker](symbol-picker.md)) |
timeframe row (`1H … 1Y`, current one lit) | chart type | Indicators, Replay, Alert | Undo, Redo. Right end, from the edge inwards: Add Chart | watchlist toggle, Pine Studio, Portfolio |
snapshot menu, chart settings | layout menu.

- Every button is a `Button` (id, glyph, label, tooltip, enabled, lit, action). `measure` lays the
  labels out, `arrange` places the two runs, `paint_button` draws one. Tests use `arrange`, so the
  geometry they check is the one drawn.
- Width: when both runs with labels do not fit, labels (Indicators, Replay, Add Chart) drop and the
  icons stay; a button without an icon (a timeframe, the layout name) keeps its text. If the runs still
  collide, the right run drops buttons from its left end (the layout menu first). Never overlap. The
  narrowest window is 960 wide and keeps everything but possibly the layout name (the icon-only
  bar needs about 910 points; it is tested at 960).
- The colours are neutral (`kit::primary` tints, `text_primary` / `text_secondary` /
  `text_tertiary`); the only colour is Replay's orange while a replay is open (state, not a control).
- `replay_bar.rs` (the strip under the toolbar while a replay is open) uses the same flat style: its
  `flat_tint` is the toolbar's hover/pressed tint at the same radius. Its one solid control, the
  start row's main action, is still inked like the label colour.

## What acts on what

- **Symbol** shows the focused market chart's name as its exchange writes it and never drops its label
  (`keep_label`) when the bar narrows; it is enabled with no chart (a pick then adds one). ⌘K / Ctrl+K
  does the same.
- **Chart type, Indicators, chart settings** act on the focused market chart
  (`ChartWorkspace::focused_market_chart`). They are disabled with none (empty tab, a portfolio card
  focused), the chart type for a line-only source, settings for a CoinMarketCap gauge. A card header has
  no chart-type button and no "+" any more; a card's title still opens its settings.
- **Replay** moved here from the tool rail (`ChartWorkspace::toggle_replay`); enabled with a market
  chart (CoinMarketCap excepted) or while a replay is open.
- **Undo / Redo** are the workspace's undo stacks (`undo.rs`), the same as the keyboard shortcut;
  disabled when the stack is empty.
- **Alert** starts a price alert on the focused chart's market (`create_alert_for_focused_chart`); it
  opens the alert editor, never saves one, and is disabled for a market with no price feed.
- **Snapshot** (camera, right of chart settings) opens the snapshot menu for the focused chart
  (`snapshot_menu::menu`). Both were buttons in each card's header, which now has neither.
- The chart-type pick goes through `set_chart_type_for_mcp`, so the toolbar, the MCP tool and the
  menu share one rule.

## Verified and not

Unit tests cover layout at several widths, that every new button asks for its action and that a
disabled one never does, and the workspace methods. `gallery_of_the_toolbar` (ignored, set
`DEGENVIEW_GALLERY_DIR`) writes pictures of the bar in each state. Not checked in a real window on
Linux or Windows.
