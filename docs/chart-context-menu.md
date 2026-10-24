# Chart context menus

What the right-click menus on a chart do, how they are built, and what is verified. The
specification this implements is `context-menu.md`; this file records what was built, what was
checked against TradingView, and what differs.

## What opens, and where

A right-click is answered by exactly one menu, chosen by what is under the pointer:

| Under the pointer | Menu |
| --- | --- |
| A drawing (Cursor tool, no gesture in progress) | The drawing menu (`ui::drawing_layer::menu`): Settings, Copy, Duplicate, style copy/paste, lock, hide, stacking order, vertices, Delete. |
| A paper order or position tag | Cancel order / Close position, Show in Paper panel. |
| A price alert tag | Edit, Pause or Resume, Delete (the pause and play icons mark the toggle). |
| A script's pane | Settings, Hide or Show, Visual order, Pine logs, Open Script Manager, Remove from chart. |
| The price axis | Reset price scale, then the four [price scale modes](price-scale.md) (Regular, Percent, Indexed to 100, Logarithmic; the active one checked). |
| The time axis | Reset chart view. |
| Empty plot | The background menu (below). |

A right-click never pans, draws, changes the tool or finishes anything else: drags and clicks that
move the chart or draw are primary-button only. With a drawing tool armed or a gesture in
progress, an empty-plot right-click opens nothing (a polyline still finishes on right-click, as
before).

### The background menu

| Entry | Does | Backend it reuses |
| --- | --- | --- |
| Copy price *p* | Copies `p` as a bare number to the system clipboard. | `PriceFormat::plain` |
| Paste ⌘V / Ctrl+V | Pastes copied drawings at the click. | `model::chart_clipboard`, `apply_drawing_commands` |
| Add alert on *SYMBOL* at *p*… | Opens the price-alert editor with the level and direction filled in. | `PriceAlertEditorState`, `SavePriceAlert` |
| Sell limit / Buy stop / Buy limit / Sell stop @ *p* | Opens the order ticket on that side and type. | `PaperPanel::open_pending_ticket`, `TicketDraft::pending` |
| Add order on *SYMBOL* at *p*… | Opens the full ticket with the price and the type the price suggests. | `PaperPanel::open_ticket` |
| Lock / Unlock vertical cursor line by time | Pins a vertical line to the clicked time. | `ChartCard::time_lock` |
| Table view | Docks the bars and indicator values as a table under the charts. | `model::chart_table`, `ui::chart_table_view` |
| Object tree | Docks the symbol, indicators, scripts and drawings as a list on the right. | `model::object_tree`, `ui::object_tree` |
| Chart template ▸ | Applies a preset or saved appearance template, saves the current look, manages templates, resets. | `ChartTemplateStore`, `ChartCard::apply_template`, the settings sheet's dialogs |
| Remove *N* drawings | Asks, then removes the chart's drawings as one undo step. | `confirm_drawings_removal`, `apply_drawing_commands` |
| Remove *N* indicators | Asks, then removes built-in indicators and scripts; undoable. | `Snapshot::Indicators` |
| Settings… | Opens the settings sheet of the clicked chart on Appearance. | `chart_settings` |

Groups are separated as in the reference screenshot; a group with nothing in it is left out, and
an entry that cannot be used is greyed with a reason on hover rather than hidden, except where the
chart cannot support it at all (no alert entry for a source without alerts, no order entries for a
chart with nothing to trade).

## The snapshot

Opening a menu captures a `ChartContext` once: the chart, its symbol and source, the time and the
price under the click and the target. The price is read through the chart's own `Projection` and
`PriceScale` (log scale included), so it is a real price whatever the scale mode reads on the axis
(never a percentage or an index), never a second formula, and it is not recomputed from where the pointer is later. Choosing an entry first
`check`s the snapshot against the live workspace: the chart must still exist and still show the
same market, and a pane, alert, order or position the menu was opened on must still be there.
Otherwise nothing is done and a message says why.

## Order types

For a clicked price *p* and the chart's last price *l* (replay-cut, as the header pills use):

| *p* vs *l* | Entries |
| --- | --- |
| above | Sell limit, Buy stop |
| below | Buy limit, Sell stop |
| within half a tick | both greyed ("Price is at the market"); Add order still opens |

Quantity is whatever the ticket would default to (blank, or 1 for whole-share instruments); the
label shows it only when there is one. Choosing an entry opens the ticket, opening the paper panel
if it is closed and creating the default paper account if none exists yet (the entries work
before the panel has ever been opened); **nothing is submitted by the menu**.

## Clipboard

Copying drawings (menu, Cmd/Ctrl+C) writes `degenview-clipboard/v1` and a JSON body to the
system clipboard and keeps the copy for the menu's Paste. Pasting (menu or Cmd/Ctrl+V) accepts
only that format: at most 256 KiB and 500 drawings, every drawing decoded and checked the way a
saved chart's rows are, no rulers, no unreadable rows. Anything else on the clipboard is ignored
silently; a damaged DegenView payload is reported. A paste gets fresh ids, is unlocked and shown,
moves every anchor by one time shift and one price shift (one price ratio on a log scale) so the
first point lands on the click and the shapes keep their relationships, and is one undo step. A
keyboard paste lands under the pointer of the chart it was last over.

## Cursor time lock

A transient, per-chart vertical line at a time. It is not a drawing and is not saved, in layouts,
or templates, and linked charts do not carry it (it is separate from the shared crosshair). It is
placed with the chart's projection, so it holds its time through pans, zooms and resizes, and is
clipped to the plot and the panes. It survives a timeframe change and is released by choosing
Unlock.

## Table view

One row per bar the chart draws, newest first (the time header flips it): Time, Open, High, Low,
Close, Volume where the source has them, then a column per visible built-in indicator and per
visible script plot. It reads the same bars the painter does, so a price-based chart type shows its
constructed bars and replay shows only revealed ones, and it writes numbers with the chart's own
price format, the compact volume, and the chart's UTC times. Only the rows on screen are laid out.

## Object tree

The symbol, the built-in indicators and scripts, and the drawings (topmost first). Click selects a
drawing (the workspace's one selection, so it highlights on the chart too); double-click opens its
settings; the row buttons hide, lock and delete; the row menu reorders and deletes. Everything goes
through the paths the chart's own controls use, so drawing changes are undoable. There is no
rename or grouping: drawings have no name and no group model. A drawing this version cannot read (from
a newer build) is a row too, "From a newer version", with only Delete: it is not drawn or selectable,
and nothing else changes it.

## Pine logs

A docked panel on the right (beside the Object tree, `workspace/pine_logs.rs`, `ui/pine_logs_view`),
opened from a script's chip menu or pane menu, "Pine logs". It reads the messages the chart card
keeps for each script's run and owns none of them. Hovering a row offers Source code and Scroll to
bar. See [Pine Logs](pine-logs.md).

## Undo

The workspace's history is one chronological stack of typed entries: a chart's drawings before a
batch, or its built-in indicators and scripts before a removal. Paste, Remove drawings and every
Object Tree drawing change are one `apply_drawing_commands` step. Removing indicators (bulk, from a
pane or from the tree) restores the indicators and the script instances on Undo. Template changes
and cursor locks are not in history. Removing a script from a chart never touches its saved source
or its alert subscriptions; a subscription whose instance is gone is marked changed by the
existing reconcile, and Undo brings the instance (same id) back.

## Dragging an alert tag

A price alert's tag can be dragged up or down to move its level, as a paper order's can
(`ui::alert_lines`). The cursor is the vertical resize arrow over the tag and for the whole drag.

- While dragged, the tag, its dashed line and its caption follow the pointer, clamped to the plot,
  and the caption shows the level it would land on (on the market's tick) and the side that level
  is on. Esc ends the drag, puts the tag back and saves nothing (egui reports that stop like a
  release, so the view checks for the key).
- The drop is a `WorkspaceAction::MoveAlert`: the alert as it is, with a new `CrossesAbove` or
  `CrossesBelow` level in its own currency (the USD drop times the app's FX rate; no rate, no
  save, and a message). The side follows the level against the market's last price, as for Add
  alert at a price, and a drop level with the market keeps the side it had. The save carries the
  last price as its baseline so the alert is armed for the side it now watches. State, frequency,
  note and webhooks are untouched: a paused alert stays paused. The app only queues the save
  (`AppAction::MoveAlert`); it does not ask for notification permission again.
- A drop lands on the market's tick and on the chart's price places (`PriceFormat::plain`), so
  the level saved is the one the caption showed, not the pointer's eight decimals.
- The tag stays at the dropped level until the alert list reports a different level (the app
  drains saves about once a second; a background agent that owns the alerts takes longer), so it
  does not jump back while the save travels the command queue. If the save never lands it returns
  to the alert's level after ten seconds.
- A percentage alert (rises or falls by) is not draggable: its level follows from its percent and
  reference. Nothing is draggable during replay.
- A tag dropped where it started saves nothing.

## TradingView: verified and not

Verified in TradingView's help centre: Table view opens from the chart's right-click menu, has a
row per point in time newest first, matches the chart, refreshes by itself and leaves hidden
indicators out; Cmd/Ctrl+C and V copy and paste drawings.

Not verifiable from the documentation, so built from the specification and marked as such:

- Which order types a clicked price offers (implemented as the table above).
- Whether Remove indicators includes scripts (it does here: a script is an indicator on this chart).
- What "lock vertical cursor line by time" persists, and whether it can be moved by clicking
  elsewhere (here: transient, and locking again replaces it).
- The exact clipboard text of Copy price (here a bare number with the chart's places, never fewer
  than two from 1 up).
- A keyboard shortcut for Add alert: the sources conflict, so none is bound or printed.

## Differences and gaps

- Copy price's label uses the chart's price format (`118,151` above 1,000), while the clipboard
  keeps the cents.
- The main series and a script's overlay plots are not separate targets; they answer with the
  background menu. A built-in indicator has a chip menu, not a plot target.
- Alerts and paper orders are disabled during replay. The paper engine itself still fills against
  the live price during a replay; that is unchanged.
- An alert's line is drawn at its level converted to USD with the app's FX rates; an alert in a
  currency with no rate is left off the chart.
- Templates keep no undo; Apply to all charts remains in the settings sheet.

## Manual check list

1. Right-click empty plot: menu opens under the pointer; near the right and bottom edges it flips;
   Esc closes it and leaves the armed tool alone.
2. Copy price, then paste into a text field: a plain number.
3. Add alert: the editor opens with the level and direction; closing it creates nothing.
4. Sell limit / Buy stop above price, Buy limit / Sell stop below: the ticket opens, no order exists.
5. Lock vertical cursor line: pan, zoom and resize; the line holds its time.
6. Table view and Object tree: open, scroll, flip order, select and hide a drawing, undo.
7. Remove drawings and Remove indicators: the count is right, one Undo restores all.
8. Two charts: every entry acts on the chart that was clicked.
9. Light and dark themes; a detached window.
10. Alert tag: the cursor is the vertical arrow; drag it, release, and it stays; the Alerts window
    shows the new level. Drag across the last price and the arrow in the caption turns. Esc
    mid-drag restores it. A paused alert stays paused. A percentage alert has no drag cursor.
11. Alert tag menu: Pause has the pause icon, Resume the play icon, and the menu is no wider than
    its longest label needs.
