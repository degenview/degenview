# Implement TradingView-Style Chart Right-Click Context Menus in DegenView (Rust)

## Objective

Implement a complete, polished, production-ready right-click context menu system for DegenView charts, closely matching TradingView Supercharts.

Use the attached TradingView screenshot as the primary visual reference and TradingView's official documentation as the behavioral reference.

The context menu must provide real, working actions—not decorative menu entries.

It must integrate with DegenView's existing:

- Chart viewport and financial coordinate transforms
- Chart settings and appearance templates
- Drawing engine
- Indicator and Pine Script management
- Price alert system
- Paper-trading engine
- Chart layouts and workspaces
- Undo/redo
- Persistence
- Native macOS/Windows/Linux keyboard conventions
- egui UI components

**Reuse existing implementations wherever possible. Refactor shared code when necessary. Do not create duplicate alert editors, order engines, drawing stores, template systems, or chart settings dialogs.**

The result should feel like TradingView's native chart context menu: immediate, contextual, compact, precise, and reliable.

---

# 1. Research TradingView's actual behavior

Before coding, study the official TradingView documentation.

Primary references:

**Context menu architecture**

https://www.tradingview.com/charting-library-docs/latest/ui_elements/context-menu/

**Table view**

https://www.tradingview.com/support/solutions/43000765410-how-to-view-chart-data-as-a-table/

**Object Tree**

https://www.tradingview.com/charting-library-docs/latest/ui_elements/object-tree/

**Remove drawings and indicators**

https://www.tradingview.com/support/solutions/43000473337-how-can-i-remove-all-drawings-indicators-from-the-chart/

**Chart layouts, indicators, and drawings**

https://www.tradingview.com/support/solutions/43000692404-layouts-charts-drawings-indicators-and-their-interaction/

**Chart templates and indicator templates**

https://www.tradingview.com/support/solutions/43000746464-getting-started-with-supercharts/

**Creating orders and alerts at a chart price**

https://www.tradingview.com/support/solutions/43000699223-how-to-create-an-order-alert-and-price-line-anywhere-on-the-chart/

**Drawing visual order**

https://www.tradingview.com/support/solutions/43000474299-how-can-i-change-the-visual-order-layer-of-indicators-drawings-candles/

**Managing alerts**

https://www.tradingview.com/support/solutions/43000595311-manage-alerts/

**Technical alerts**

https://www.tradingview.com/support/solutions/43000763315-getting-started-with-technical-alerts/

Research additional official references for:

- Copy price
- Paste drawings
- Lock vertical cursor line by time
- Chart template context-menu behavior
- Object Tree interactions
- Trading order context menus
- Keyboard shortcuts
- Chart settings from the context menu
- Context menus on drawings, indicators, axes, and order lines

Not every detail in the screenshot has a dedicated official documentation page. Where documentation is incomplete, compare against TradingView's actual UI and document the observed behavior.

Do not invent TradingView behavior or claim exact parity without verification.

---

# 2. Inspect DegenView's existing architecture

Read the attached Rust architecture and inspect the repository before making changes.

Relevant files:

### Chart and interaction

- `src/ui/chart_card.rs`
- `src/workspace.rs`
- `src/model/canvas.rs`
- `src/ui/canvas_render.rs`
- `src/ui/chart_buttons.rs`
- `src/ui/chart_settings.rs`
- `src/ui/popover.rs`
- `src/ui/kit.rs`
- `src/ui/theme.rs`
- `src/ui/icons.rs`
- `src/ui/accessibility.rs`

### Drawings

- `src/model/drawings.rs`
- `src/ui/drawing_layer.rs`
- `src/ui/drawing_render.rs`
- `src/ui/tool_sidebar.rs`

### Indicators and Pine

- `src/model/indicators.rs`
- `src/ui/chart_indicator_strip.rs`
- `src/pine_host.rs`
- `src/ui/pine_chart_render.rs`
- `src/model/pine_instance.rs`

### Alerts

- `src/model/alert.rs`
- `src/alert_host.rs`
- `src/ui/alert_editor.rs`
- `src/ui/alert_center.rs`

### Paper trading

- `src/model/paper.rs`
- `src/model/paper_ticket.rs`
- `src/model/paper_engine.rs`
- `src/paper_host.rs`
- `src/ui/paper_order_ticket.rs`
- `src/ui/paper_chart_trading.rs`

### Chart templates and layouts

- `src/model/chart_template.rs`
- `src/store/chart_template.rs`
- `src/ui/chart_templates.rs`
- `src/model/layout.rs`
- `src/model/persisted.rs`
- `src/model/saved_view.rs`

### Persistence and application

- `src/store/mod.rs`
- `src/app.rs`
- `src/native_bridge.rs`

Use the actual repository as the source of truth.

Before implementing, identify existing right-click behavior, chart command handlers, context menus, keyboard shortcuts, and clipboard helpers.

Do not override existing object-specific context menus accidentally.

---

# PART A — CONTEXT MENU INFRASTRUCTURE

## 3. Implement a shared chart context-menu system

Build a reusable, typed context-menu infrastructure.

The system must support different targets:

- Empty chart background
- Main price series
- Drawing object
- Indicator
- Pine Script visual
- Price scale
- Time scale
- Alert line
- Paper order line
- Paper position marker

The attached screenshot primarily shows the **chart background menu**.

Implement that menu completely, while structuring the system so other context-specific menus can reuse the same infrastructure.

Do not show the background menu indiscriminately when the user right-clicks an editable drawing or an order.

### Suggested context model

```rust
struct ChartContext {
    workspace_id: WorkspaceId,
    chart_id: ChartId,
    pane_id: PaneId,

    screen_position: Pos2,
    chart_position: Pos2,

    timestamp: Option<i64>,
    price: Option<f64>,

    target: ChartContextTarget,
}
```

Illustrative only; adapt to the existing types.

The context must capture the financial coordinate at the time of right-click.

Do not recompute the target price from the current mouse position when a menu item is selected later.

---

## 4. Context-target priority

Resolve right-click targets in a deterministic order.

Suggested priority:

1. Modal or active interaction that owns the pointer.
2. Drawing edit handle.
3. Drawing body.
4. Alert/order/position marker.
5. Indicator or Pine visual with an interactive target.
6. Price or time axis.
7. Chart plot background.

Respect the existing hit-testing infrastructure.

Avoid duplicating geometry calculations.

If multiple objects overlap, use the established selection/z-order rules.

Right-click must not accidentally begin a drawing, pan the chart, or change the selected tool.

---

## 5. Context snapshot lifetime

The menu should retain a stable context while open.

For example:

1. User right-clicks at BTCUSD price 118,150.99.
2. Menu opens.
3. User moves the mouse to the Add Alert entry.
4. User clicks.
5. Alert editor opens with 118,150.99.

The price must not change merely because the pointer moved.

The same applies to:

- Symbol
- Chart identity
- Timestamp
- Pane
- Target drawing
- Target indicator
- Target order

Before executing a command, verify that its target still exists and remains eligible.

If the chart was closed or the target removed while the menu was open, fail safely.

---

## 6. Centralized command dispatch

Define typed context-menu commands.

For example:

```rust
enum ChartContextCommand {
    CopyPrice,
    Paste,
    AddPriceAlert,
    OpenSellLimitTicket,
    OpenBuyStopTicket,
    OpenOrderTicket,
    ToggleCursorTimeLock,
    OpenTableView,
    OpenObjectTree,
    OpenChartTemplates,
    RemoveDrawings,
    RemoveIndicators,
    OpenChartSettings,
}
```

Expand as necessary for object-specific menus.

The UI should emit commands; the existing application/workspace owners should execute them.

Do not perform direct SQLite writes or order-engine mutations inside menu rendering code.

Reuse existing action handlers.

---

# PART B — VISUAL DESIGN

## 7. Match the supplied screenshot

Reproduce the screenshot's overall visual hierarchy.

The menu uses:

- Dark floating surface
- Rounded corners
- Subtle shadow
- Comfortable horizontal padding
- Left-aligned labels
- Optional leading icons
- Right-aligned keyboard shortcuts
- Thin separators
- Consistent row heights
- Submenu chevrons
- Hover highlighting
- Disabled-item styling

The screenshot's groups are:

### Clipboard

- Copy price
- Paste

### Alerts and trading

- Add alert
- Sell limit
- Buy stop
- Add order

### Cursor

- Lock vertical cursor line by time

### Chart utilities

- Table view
- Object tree
- Chart template

### Cleanup

- Remove drawings
- Remove indicators

### Settings

- Settings

Use the screenshot for proportions and grouping, but adapt dimensions to DegenView's existing design system.

Do not hardcode the screenshot's pixel dimensions.

Support Retina scaling and different display sizes.

---

## 8. Menu placement

The menu should open near the right-click location.

Requirements:

- Do not clip outside the application window.
- Flip horizontally near the right edge.
- Flip vertically near the bottom edge.
- Remain fully accessible on smaller screens.
- Support scrolling when the menu cannot fit vertically.
- Keep submenus inside available window bounds.
- Avoid overlapping the macOS title-bar controls.
- Work in detached native windows.

Do not reposition the underlying chart to make room.

---

## 9. Keyboard and pointer behavior

Support:

- Right-click to open.
- Left-click item to execute.
- Hover to highlight.
- Arrow keys to navigate.
- Enter/Space to activate.
- Escape to dismiss.
- Click outside to dismiss.
- Keyboard shortcut labels.
- Accessible focus indicators.

Submenus should support pointer movement without closing prematurely.

Avoid hover flicker when moving diagonally toward a submenu.

Do not require pixel-perfect pointer movement to reach submenu entries.

---

# PART C — COPY PRICE

## 10. Copy price

Implement the first screenshot action:

    Copy price 118,150.99

The displayed price must come from the financial coordinate under the right-click pointer.

### Requirements

- Use the active chart pane's price transform.
- Respect linear/logarithmic/inverted scales.
- Use the instrument's existing precision and formatting rules.
- Display the price in the menu label.
- Copy a clean machine-readable numeric value to the system clipboard.
- Do not include thousands separators in the copied value unless TradingView reference behavior requires them.
- Do not copy a stale crosshair price.

Example:

Displayed:

    Copy price 118,150.99

Clipboard:

    118150.99

Confirm the precise clipboard formatting against TradingView.

### Invalid contexts

If the clicked pane has no meaningful price coordinate, omit or disable Copy price.

Do not invent a price.

---

# PART D — PASTE

## 11. Paste

Implement Paste using DegenView's existing drawing and clipboard systems.

TradingView's Paste action is context-dependent.

Research exactly which objects TradingView allows users to copy and paste.

### Required DegenView behavior

At minimum, support copying and pasting supported user-created drawing objects.

Where existing clipboard functionality already supports additional object types, reuse it.

Requirements:

- Copy drawing geometry and styles.
- Paste as a new drawing with a new stable ID.
- Preserve the drawing type.
- Preserve its financial geometry.
- Support placement relative to the clicked chart coordinate where appropriate.
- Preserve relationships between multiple copied drawings.
- Do not overwrite the source drawing.
- Make paste undoable.
- Persist the pasted drawing normally.

### Cross-chart paste

If pasting into another chart:

- Resolve the target chart.
- Preserve meaningful financial coordinates.
- Handle different symbols/timeframes.
- Do not silently reinterpret invalid pane-specific values.

### Clipboard safety

Do not deserialize arbitrary system clipboard data directly into trusted internal structures without validation.

Use a typed, versioned application clipboard format.

Reject malformed, oversized, or unsupported payloads safely.

### Disabled state

Disable Paste if no supported clipboard object is available.

Show the platform-appropriate shortcut:

- macOS: Command+V
- Windows/Linux: Ctrl+V

---

# PART E — ADD ALERT

## 12. Add alert at clicked price

Implement:

    Add alert on BTCUSD at 118,150.99...

The symbol and price must reflect the clicked chart.

Clicking this item opens DegenView's existing price-alert editor.

Prefill:

- Instrument
- Data source
- Target price
- Relevant condition
- Existing default alert settings

### Critical behavior

Opening the editor must not immediately create an alert.

The user must review and save the alert.

Reuse:

- `AlertHost`
- `PriceAlertEditorState`
- Existing validation
- Existing notification settings
- Existing webhook integration
- Existing persistence

Do not implement a second alert editor.

### Price conditions

Research TradingView's default condition when an alert is created from the chart.

Where DegenView supports crossing above/below, choose an appropriate default based on the clicked price and current market price.

If the reference behavior cannot be confirmed, use the application's existing alert default.

Do not silently choose an unsupported alert type.

### Shortcut

Display the verified shortcut for the platform.

Do not register a shortcut that conflicts with an existing action.

---

# PART F — PAPER-TRADING ACTIONS

## 13. Trading context

The screenshot shows:

    Sell 1 BTCUSD @ 118,150.99 limit
    Buy 1 BTCUSD @ 118,150.99 stop
    Add order on BTCUSD at 118,150.99...

These entries depend on the clicked price relative to the current market price.

Research TradingView's order-type selection logic.

Do not hardcode Sell Limit and Buy Stop for every clicked price.

---

## 14. Determine order types from market context

Use the current executable market reference where available.

For a normal order model:

- A sell limit is generally placed above the current market.
- A buy stop is generally placed above the current market.
- A buy limit is generally placed below the current market.
- A sell stop is generally placed below the current market.

TradingView's exact menu labels and availability may depend on broker, bid/ask, instrument, and order settings.

Research and reproduce the appropriate rules.

### Requirements

- Use the clicked price.
- Use the chart's actual instrument.
- Use the active paper-trading account.
- Use supported order types.
- Respect tick size.
- Respect quantity precision.
- Respect available balance/margin.
- Respect account restrictions.

Do not assume quantity 1 is valid for every instrument.

Use the existing ticket's last-used/default quantity where appropriate.

---

## 15. Quick Sell/Buy actions

The screenshot suggests quick-order entries.

For DegenView:

- Open the existing paper-trading ticket prefilled with the relevant side, type, quantity, and price.
- Require the normal confirmation or submission action.
- Reuse the existing order validation and risk checks.

Do not immediately submit an order merely because the context-menu entry was clicked.

If the application already has an explicitly enabled one-click trading mode, respect its existing permission and confirmation rules.

Never route these actions to live brokerage execution unless the user has explicitly configured and authorized that capability.

---

## 16. Add order

Implement:

    Add order on BTCUSD at 118,150.99...

This should open the existing full order ticket with:

- Instrument preselected
- Clicked price prefilled
- Appropriate order type suggested
- Quantity editable
- Side editable
- Order type editable
- Stop/limit controls as supported
- Risk/validation messages

The menu action must not create a draft order in persistent trading state until the existing order workflow requires it.

---

## 17. Trading availability

If paper trading is unavailable for a chart:

- Do not show misleading executable order actions.
- Disable them with a clear reason or omit them according to the verified TradingView behavior.
- Preserve the alert and chart utilities.

For unsupported instruments such as certain indices or nontradable data sources, do not invent order support.

---

# PART G — LOCK VERTICAL CURSOR LINE

## 18. Lock vertical cursor line by time

Implement the screenshot action:

    Lock vertical cursor line by time

Research TradingView's exact interaction.

The intended functionality is to preserve a vertical time reference on the chart after the pointer moves.

### Requirements

- Capture the clicked chart timestamp.
- Render a vertical cursor line at that time.
- Keep the line attached to time, not a fixed screen X coordinate.
- Preserve the lock when horizontally panning.
- Preserve it when zooming.
- Preserve it when resizing the chart.
- Support unlocking.
- Show the corresponding time-axis label.
- Respect chart pane clipping.

### Interaction

When unlocked, show:

    Lock vertical cursor line by time

When locked, show the appropriate unlock action.

Verify whether TradingView supports moving or replacing the lock by clicking elsewhere.

### Linked charts

DegenView supports optional synchronization of horizontal chart position and zoom.

Do not automatically synchronize cursor locking unless the application already has a separate synchronized-crosshair setting.

Keep these concepts distinct.

### Persistence

Research whether TradingView treats the lock as transient or persistent.

Follow the closest documented behavior.

Do not accidentally save it as an ordinary drawing.

---

# PART H — TABLE VIEW

## 19. Implement TradingView-style Table view

TradingView's Table view is a structured representation of chart data.

Official documentation specifies:

- One row per point in time.
- Values consistent with the chart.
- Latest observations first.
- Active visible indicators included.
- Hidden indicators excluded.
- Automatic updates.

This must be a functional data view, not a screenshot or static table.

### UI

Clicking Table view should open or switch to a table representation associated with the clicked chart.

Use the existing DegenView layout and component conventions.

### Columns

At minimum, support applicable columns:

- Date/time
- Open
- High
- Low
- Close
- Volume
- Visible built-in indicator values

Where feasible, include supported Pine plot outputs.

Do not show meaningless OHLC columns for non-OHLC data sources.

### Sorting

Default to newest first.

Allow useful sorting behavior where appropriate.

### Formatting

Use the same:

- Price precision
- Percentage formatting
- Volume formatting
- Timezone
- Date/time formatting

as the chart.

### Performance

Use virtualized rows or equivalent efficient egui scrolling for large histories.

Do not construct a widget for every historical candle on every frame.

### Live updates

The table must update when:

- New candles arrive.
- The forming candle changes.
- The timeframe changes.
- The symbol changes.
- Visible indicators change.
- Replay advances.

### Replay

Never expose unrevealed future candles in Table view.

### Synthetic charts

Use the active chart's displayed series semantics where appropriate.

Do not silently substitute raw OHLC candles for synthetic bars without clearly defining the behavior.

---

# PART I — OBJECT TREE

## 20. Implement TradingView-style Object Tree

The Object Tree provides a structured view of chart objects.

TradingView documents it as a way to manage drawings, indicators, and symbols.

### Requirements

Show a hierarchical list containing relevant:

- Main chart symbol
- Built-in indicators
- Pine Script instances
- User-created drawings
- Other supported chart objects

### Object actions

Support, where applicable:

- Select
- Show/hide
- Lock/unlock
- Rename
- Edit settings
- Delete
- Reorder
- Bring forward/send backward
- Group/ungroup drawings where supported

### Selection synchronization

Selecting a drawing in Object Tree should select it on the chart.

Selecting a drawing on the chart should highlight the corresponding tree entry.

Do not maintain a separate inconsistent selection model.

### Persistence

Use the existing drawing, indicator, and Pine persistence.

Object Tree is a view/controller over existing state, not another data store.

### Large object lists

Support scrolling, search/filtering where useful, and stable ordering.

Do not rebuild expensive object descriptions unnecessarily.

---

# PART J — CHART TEMPLATE SUBMENU

## 21. Chart template

The screenshot contains:

    Chart template >

DegenView already has a chart appearance/template system.

Reuse:

- `model/chart_template.rs`
- `store/chart_template.rs`
- `ui/chart_templates.rs`
- Existing `ChartCard::apply_template` behavior

### Important compatibility distinction

TradingView uses the term "template" in several contexts, including indicator templates and chart settings.

Do not confuse:

1. Chart appearance templates.
2. Indicator templates.
3. Drawing style templates.
4. Saved workspace layouts.

For DegenView, the screenshot's Chart template submenu should integrate with the existing chart appearance templates unless reference testing establishes a different intended behavior.

If implementing TradingView-style indicator templates as well, expose them with clear naming rather than silently changing the meaning of existing appearance templates.

### Submenu

Provide:

- Available saved templates
- Apply template
- Save current chart appearance as template
- Manage templates
- Reset to default where appropriate

Reuse the existing template dialogs and validation.

### Safety

Applying an appearance template must not remove:

- Drawings
- Indicators
- Pine scripts
- Alerts
- Orders
- Symbol
- Timeframe

unless the existing template's explicitly documented scope includes that property.

Do not treat a saved layout as an appearance template.

---

# PART K — REMOVE DRAWINGS

## 22. Remove drawings

Implement:

    Remove 17 drawings

The count must be computed from the clicked chart's actual eligible drawing collection.

Do not hardcode the screenshot count.

### Requirements

- Count persistent user drawings.
- Exclude transient ruler previews.
- Exclude crosshair lines.
- Exclude canceled drafts.
- Respect chart ownership.
- Handle hidden drawings correctly.
- Handle locked drawings according to the established deletion policy.

The label should update when drawings are added or removed.

Examples:

    Remove 1 drawing
    Remove 17 drawings

### Confirmation

For destructive bulk removal, use a polished confirmation flow consistent with DegenView's existing UX.

Show the number of affected drawings.

Do not remove drawings from other charts unless the user explicitly selected a broader scope.

### Undo

Reuse DegenView's existing drawing undo stack.

Bulk removal must be **one undoable operation**, not 17 separate undo steps.

### Persistence

Use the existing chart drawing mutation and save paths.

Do not rewrite unrelated chart state.

---

# PART L — REMOVE INDICATORS

## 23. Remove indicators

Implement:

    Remove 5 indicators

Count the chart's actual active indicators.

Research TradingView's treatment of built-in indicators versus applied Pine scripts in this action.

Define and document DegenView's scope accordingly.

### Requirements

- Correct live count.
- Remove the intended indicators.
- Remove associated indicator panes when no longer needed.
- Clean up associated rendering state.
- Preserve main chart data.
- Preserve user drawings in the main price pane.
- Preserve alerts and trading state unless a real dependency requires otherwise.

### Pine scripts

If applied Pine scripts are included in TradingView's equivalent Remove Indicators action, implement that behavior through the existing Pine instance lifecycle.

Do not delete the underlying saved Pine source files.

Do not delete Pine alert subscriptions merely because the visible chart instance was removed unless the application's existing semantics explicitly require it.

### Confirmation

Use confirmation for destructive bulk operations where appropriate.

### Undo

Investigate existing indicator undo support.

If it exists, reuse it.

If not, implement a coherent undoable chart-configuration transaction where practical rather than misusing the drawing undo stack.

Do not claim the action is undoable unless it really is.

---

# PART M — SETTINGS

## 24. Open chart settings

Implement:

    Settings...

This must open the existing settings sheet for the clicked chart.

Reuse `ui/chart_settings.rs`.

The sheet already contains:

- Indicators
- Scripts
- Appearance
- Canvas
- Templates

Open the appropriate default tab consistent with TradingView behavior and DegenView's existing UX.

Do not create another chart settings window.

### Chart identity

In multi-chart layouts, Settings must open for the chart that was right-clicked, not whichever chart happened to be active previously.

### Keyboard shortcut

Research TradingView's shortcut and check for conflicts with existing DegenView shortcuts.

---

# PART N — OBJECT-SPECIFIC CONTEXT MENUS

## 25. Drawings

When right-clicking a drawing, show a drawing-specific menu.

Possible actions:

- Edit properties
- Duplicate
- Copy
- Paste style
- Lock/unlock
- Hide/show
- Visual order
- Delete

Reuse the drawing system's existing actions.

Do not expose unsupported operations as clickable placeholders.

---

## 26. Indicators and Pine instances

When right-clicking an indicator or Pine visual, show relevant actions:

- Settings
- Hide/show
- Remove
- Visual order
- Add alert where supported
- Open Pine source where appropriate

Reuse the existing indicator strip, Pine instance settings, and Script Manager.

Do not remove a Pine script's source file when removing its chart instance.

---

## 27. Alert lines

When right-clicking an alert marker or line, provide relevant actions:

- Edit alert
- Pause/resume
- Delete
- Show/hide extended line where supported

Reuse `AlertHost` and existing alert editor commands.

Do not change alert state through ad hoc direct SQLite mutations.

---

## 28. Paper orders and positions

When right-clicking a paper-trading order or position overlay, show relevant actions:

- Edit eligible pending order
- Cancel eligible pending order
- View order details
- View position
- Close position through the existing ticket/workflow where supported

Respect order lifecycle and account permissions.

Do not offer Cancel on an already filled order.

Do not silently execute trading actions without the normal validation.

---

## 29. Price and time scales

Research TradingView's scale-specific context menus.

Reuse existing chart scale settings and navigation controls.

At minimum, preserve existing:

- Autoscale
- Manual scale behavior
- Scale reset
- Time navigation
- Relevant scale configuration

Do not replace a scale-specific menu with the chart background menu.

---

# PART O — SHORTCUTS

## 30. Keyboard shortcut integration

The screenshot shows shortcuts beside several actions.

Research their current TradingView equivalents.

Support platform-appropriate shortcuts where they do not conflict with existing DegenView commands.

Requirements:

- macOS Command symbols.
- Windows/Linux Ctrl equivalents.
- Correct Shift/Alt/Option rendering.
- One authoritative shortcut registry where practical.
- Shortcut labels generated from actual bindings.
- Disabled commands not triggered by shortcuts.
- Correct focus behavior when editing text.

Do not hardcode shortcut labels that are not actually implemented.

---

# PART P — MULTI-CHART CORRECTNESS

## 31. Target the clicked chart

DegenView supports multiple charts per workspace.

Every context-menu command must operate on the chart where the menu was opened.

Test:

- Two charts with the same symbol.
- Two charts with different symbols.
- Different timeframes.
- Different data providers.
- Different chart types.
- Multiple native windows.
- Multiple workspace tabs.

Do not rely solely on the globally active chart if the right-click occurred on another chart.

---

## 32. Viewport correctness

The menu's captured price and timestamp must remain accurate after:

- Horizontal panning.
- Vertical panning.
- Zooming.
- Autoscale changes.
- Logarithmic scale changes.
- Chart resizing.
- Linked-chart synchronization.

Use the same authoritative viewport transforms as the chart renderer.

Do not introduce another price-to-pixel formula inside the menu code.

---

## 33. Replay compatibility

During replay:

- Copy price must use the visible replay chart.
- Add Alert must respect existing alert restrictions.
- Table view must not reveal future candles.
- Object Tree must show the current chart objects.
- Drawing operations must preserve replay semantics.
- Paper orders must follow the existing replay/paper-trading rules.

Do not expose hidden future market data through context-menu actions.

---

# PART Q — PERSISTENCE AND STATE

## 34. Reuse existing persistence

The context menu is a command surface, not a new persistence owner.

Use the existing storage for:

- Drawings
- Indicators
- Pine instances
- Alerts
- Orders
- Chart templates
- Chart settings
- Layouts

Do not create a generic `context_menu_state` database table for objects that already have owners.

Persist only genuine new user preferences where necessary.

---

## 35. Undo and redo

Route mutations through the correct existing undo mechanism.

Examples:

- Paste drawing: drawing undo.
- Remove drawings: one drawing undo transaction.
- Change drawing visual order: drawing undo where supported.
- Remove indicators: chart-configuration undo where supported.
- Change chart template: existing appearance/template transaction.

Do not put chart navigation or context-menu opening into drawing history.

Do not treat order submission or alert triggering as ordinary reversible drawing edits.

---

# PART R — PERFORMANCE AND RELIABILITY

## 36. Menu performance

Opening the menu should feel immediate.

Do not:

- Fetch network data.
- Recompile Pine scripts.
- Query large SQLite collections synchronously.
- Recompute every indicator.
- Rebuild the entire drawing geometry cache.
- Block on a long mutex hold.

Use existing in-memory chart state for labels, counts, and eligibility.

Resolve expensive work only when the user executes the relevant action.

---

## 37. Robust error handling

Handle:

- Chart removed while menu is open.
- Drawing deleted while menu is open.
- Invalid price coordinate.
- Missing symbol.
- Missing current quote.
- Unsupported order type.
- No paper account.
- Empty clipboard.
- Corrupt clipboard data.
- Failed persistence.
- Unsupported chart data.
- Replay restrictions.
- Native window closing.
- Stale selected object.

Show actionable error feedback.

Do not panic, silently succeed, or mutate the wrong chart.

---

# PART S — TESTING

## 38. Unit tests

Add deterministic tests for:

- Context-target resolution.
- Price coordinate capture.
- Price formatting.
- Clipboard formatting.
- Command eligibility.
- Menu label generation.
- Drawing count.
- Indicator count.
- Order-type suggestions.
- Platform shortcut formatting.
- Menu group visibility.
- Context snapshot validity.
- Chart identity routing.

---

## 39. Integration tests

### Background menu

1. Open a chart.
2. Right-click the plot.
3. Verify the menu opens at the pointer.
4. Verify the displayed symbol and price.
5. Copy price.
6. Verify clipboard content.
7. Open Add Alert.
8. Verify the alert editor's prefilled values.
9. Open Add Order.
10. Verify the order ticket's prefilled values.
11. Open Settings.
12. Verify the correct chart is targeted.

### Drawings

1. Create multiple drawings.
2. Open the menu.
3. Verify the correct drawing count.
4. Remove drawings.
5. Verify all intended drawings are removed.
6. Undo.
7. Verify they are restored.
8. Save/reopen.
9. Verify persistence.

### Indicators

1. Add built-in indicators and Pine instances.
2. Open the menu.
3. Verify the count.
4. Open Object Tree.
5. Verify entries.
6. Open Table view.
7. Verify visible indicator values.
8. Remove indicators.
9. Verify correct lifecycle behavior.

### Multi-chart

1. Open four charts.
2. Right-click chart C.
3. Copy its price.
4. Create an alert.
5. Open settings.
6. Verify every action targets chart C.
7. Verify charts A, B, and D remain unchanged.

---

## 40. Visual tests

Compare the implementation with the screenshot.

Check:

- Menu width and spacing.
- Rounded corners.
- Background.
- Text color.
- Icons.
- Separator positions.
- Shortcut alignment.
- Hover states.
- Disabled states.
- Submenu positioning.
- Screen-edge behavior.
- Dark/light theme.
- Retina scaling.

Use existing screenshot testing infrastructure if available.

Otherwise provide a repeatable manual visual test checklist.

---

## 41. Accessibility tests

Verify:

- Keyboard navigation.
- Screen-reader labels.
- Focus management.
- Escape behavior.
- High-contrast readability.
- Appropriate disabled-state announcements.
- Logical submenu navigation.
- Correct shortcut descriptions.

The menu must remain usable without a mouse where the platform permits keyboard invocation.

---

# PART T — IMPLEMENTATION ORDER

## 42. Recommended sequence

1. Audit current right-click behavior and command paths.
2. Research TradingView's context menu and object-specific variants.
3. Define the context snapshot and target-resolution model.
4. Define typed commands and eligibility rules.
5. Implement the shared menu UI.
6. Implement Copy price.
7. Integrate Paste with the existing drawing clipboard.
8. Integrate Add Alert.
9. Integrate paper-trading quick actions and Add Order.
10. Implement cursor time locking.
11. Implement Table view.
12. Implement Object Tree.
13. Integrate Chart template submenu.
14. Implement Remove Drawings.
15. Implement Remove Indicators.
16. Integrate Chart Settings.
17. Add drawing/indicator/alert/order-specific menus.
18. Integrate keyboard shortcuts.
19. Test multi-chart and multi-window behavior.
20. Test replay and synthetic charts.
21. Complete visual regression and accessibility checks.
22. Profile and polish the result.

Complete each action end-to-end before marking it finished.

Do not ship a menu where half the entries open placeholder dialogs.

---

# PART U — ACCEPTANCE CRITERIA

The implementation is complete when:

- [ ] Right-clicking the chart opens a polished contextual menu.
- [ ] The menu closely matches the supplied TradingView screenshot.
- [ ] The correct chart, price, time, and target are captured.
- [ ] Copy price works.
- [ ] Paste works for supported copied objects.
- [ ] Add Alert opens the existing editor with correct values.
- [ ] Trading quick actions use the existing paper-trading ticket.
- [ ] Order types reflect the clicked price and market context.
- [ ] Add Order opens the correct ticket.
- [ ] No live or paper order is submitted accidentally.
- [ ] Vertical cursor time locking works.
- [ ] Table view is fully functional.
- [ ] Object Tree is fully functional.
- [ ] Chart template submenu uses existing templates.
- [ ] Remove Drawings uses correct counts and undo.
- [ ] Remove Indicators uses correct counts and lifecycle handling.
- [ ] Settings opens for the correct chart.
- [ ] Object-specific context menus work.
- [ ] Shortcut labels match real bindings.
- [ ] Submenus work without hover flicker.
- [ ] Menu placement handles window edges.
- [ ] Dark/light themes are polished.
- [ ] Multiple charts remain isolated.
- [ ] Replay does not expose future data.
- [ ] Persistence remains correct.
- [ ] No duplicated alert/order/template systems are introduced.
- [ ] Tests pass.
- [ ] Performance remains responsive.

---

# PART V — FINAL REPORT

After implementation, provide:

1. Files changed.
2. TradingView behaviors researched.
3. Context-menu architecture.
4. Implemented menu actions.
5. Object-specific menu support.
6. Existing systems reused.
7. Clipboard behavior.
8. Alert and paper-order safety.
9. Table view and Object Tree capabilities.
10. Template integration.
11. Undo/redo and persistence.
12. Keyboard shortcuts.
13. Visual comparison results.
14. Automated test results.
15. Remaining TradingView differences.

Clearly distinguish fully implemented behavior from approximations or unsupported functionality.

Do not claim production readiness if actions target the wrong chart, copy the wrong price, submit unintended orders, lose drawings, delete Pine source files, or expose future replay data.

**The final result must be a context-sensitive, fully functional TradingView-style chart command system that feels native to DegenView and reuses its existing Rust architecture.**