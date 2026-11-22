# Chart cursors

The cursor menu on the tool rail's first tile: Cross, Dot, Arrow, Demonstration and the Eraser.
There is no Magic cursor and none is planned.

A cursor is the pointer's resting state, not a drawing tool. It lives beside the armed `Tool`:
arming a drawing tool or Zoom In puts the cursor down (the cross is in force then, because placing
an anchor needs the lines and the tags), and Esc or the tile brings it back.

## Design

- **Model.** `model::cursor`: `CursorMode` and `CursorMode::caps()`, one row of plain flags
  (`lines`, `axis_tags`, `readout`, `pointer`). Painters and gestures read a capability; nothing
  matches on the mode, so changing a mode's behaviour is one edit in `caps`.
- **State.** `DrawingTools::cursor` is the workspace's (the tab's) cursor, shared by every chart in
  it. `DrawingTools::cursor()` is the one in force; `chosen_cursor()` the one the user picked.
  `choose_cursor` also puts any armed tool and Zoom In down. It is pushed to every card each frame
  (`ChartCard::set_cursor_mode`, like `set_zoom_armed`); it is never part of a chart, a template or a
  saved layout, and switching it moves nothing (viewport, symbol, timeframe, indicators, drawings,
  replay and sync stay as they were).
- **Persistence.** `DrawingPrefs::cursor` in the synced `drawing.prefs` setting, saved when it
  changes (never per pointer frame). A value this build does not know falls back to Cross and keeps
  the other options. Only a resting cursor (`CursorMode::persists`: Cross, Dot, Arrow) is stored:
  Demonstration and the Eraser are modes of the moment, so a restart (or another Mac) never opens in
  a mode that marks the chart or deletes. Esc (after Zoom In and an armed tool) puts them down, back
  to the resting cursor.
- **One crosshair.** `ChartCard::draw_crosshair` is still the only crosshair; it asks the caps which
  of its parts to draw (lines, price and time tags, the box naming the bar). Hover reporting is
  unchanged, so linked charts, replay and compared markets behave as before. Other charts' shared
  column follows the same caps, since the mode is the tab's.
- **Pointer.** `ChartCard::set_cursor` (`navigation.rs`) maps the caps' `PointerShape` to the system
  pointer over the plot, only when nothing else chose one (a drawing's Move or Grab, an alert tag):
  Cross the system crosshair, Arrow the platform arrow, Dot hides the system pointer
  (`CursorIcon::None`). The axes keep their resize arrows and a pan its grabbing hand.
- **Dot.** egui has no custom cursor image, so the Dot is painted (`card/cursor.rs`): a 3 pt filled
  circle with a 1 pt ring in the background colour, at the live pointer position (not the crosshair's
  position, which is a frame behind). Only when the dot hid the pointer on the last frame, only the
  chart the pointer is over, only on the plot (a script pane has no crosshair to mark, so it keeps
  the system pointer). Live only; snapshots never contain it.
- **Menu.** `ui/workspace/cursor_menu.rs`, opened by a click on the tile (`tool_sidebar::cursor_tile`).
  Width is measured from the labels (`cursor_menu::width`), never guessed; the picked row is a light
  plate with dark text so it reads in both themes.

## The Eraser

`ui/drawing/layer/erase.rs`. The mode (`DrawingFrame::erase`, from `caps.erases`) takes the drawing
layer's pointer in place of selecting, editing and drawing:

- A press on the plot starts a **sweep** at the position of the press, read from the frame's own
  events (so a move that arrives with the press loses nothing, and a click or flick whose press and
  release share one frame still erases; one that ends on an overlay widget, an alert tag or an order
  pill, is that widget's, not erased); every drawing the pointer passes over is marked (drawn at
  30 % opacity) and the mark is listed once. Letting go removes them all as one batch of
  `DrawingCommand::Remove`, so the whole sweep is **one undo step**, and a click is a sweep of one.
  Nothing is removed while the button is down, so Esc cancels a sweep (and the rest of that press does
  not pan). A sweep is also dropped when the window loses focus or when the card was not drawn
  (another tab showed), so a button let go while away erases nothing on return.
- What it can reach is what selection can pick: the same `hit_test` (stroke tolerance, hidden,
  hide-all, other-timeframe and `Unreadable` rows are out of reach). Candles, indicators, scripts'
  plots and drawings, alerts and paper or live orders are not drawings and cannot be touched. A
  **locked** drawing is protected: the pointer turns to "not allowed" over it and a sweep passes it.
- Each frame tests the pointer's way since the last one every 4 px at most, however far it moved,
  and tests it once more on the frame the button is let go (the last move can arrive with the
  release), so a quick drag cannot step over a thin line. The work is bounded by the plot: the
  pointer is clamped to it.
- A drag on the plot never pans (the layer claims it, even over empty space); the axes, the wheel
  and a pinch still work. Right-click opens the chart's menu, not a drawing's. The selection is
  cleared when the Eraser is picked.
- It acts only when the plot itself is hovered, so the alert tags, order pills and buttons drawn over
  the plot (own widgets, later in the frame) keep their presses.
- It removes through `ChartWorkspace::apply_drawing_commands` like every other delete, so undo, the
  layout's autosave and the iCloud tombstone per drawing are inherited. iCloud's mass-delete guard
  (at least 5 drawings and over half of a kind in one write) can hold a very large sweep for
  confirmation.

## Demonstration

`ui/chart/card/demonstration.rs` over the pure store `model::cursor::demo::Marks`. For explaining a
chart, as TradingView's help describes it: **Alt (Option) + click** puts a dot, **press and drag**
draws freehand, and what was drawn **fades away after a few seconds**.

- A stroke takes the pointer's last position on the frame the button is let go (that frame can carry
  the end of the move), so a quick stroke reaches where the hand stopped. It stays whole while the
  button is down; once finished it holds 2 s and fades over 1 s
  (**ours**: TradingView says only "a few seconds"). The fade follows the clock (`input.time`), not
  the frames; frames are requested every 16 ms only while a stroke is drawn or fading, and one at the
  end of the hold otherwise, so an idle chart costs nothing. At most 48 marks, 4096 points a stroke.
- Marks are pinned to time and price through `Projection::anchor`/`point` (the drawings' own
  mapping), so a wheel zoom during the fade keeps them on their bars. They are painted **after
  the chart has painted** (`body_inner`, once `paint_frame` returns: the input pass runs before it,
  so anything drawn there would sit under the candles and a script's `bgcolor`), above the chart and
  its overlays and clipped to the plot, so they are **not drawings**: never in
  `Chart::drawings`, a command, an undo step, a layout, an iCloud record, a template or a
  snapshot (`paint_snapshot` never sees them; a test pins it).
- The cursor takes the plot drag (no pan while a stroke is drawn; the axes, the wheel and the
  pinch work), hides the crosshair lines, tags and the box naming the bar, keeps the plain arrow, and
  leaves the drawings out of reach (`DrawingFrame::is_inert`: nothing is hovered, picked, moved or
  deleted). Right-click still opens the chart's menu.
- A range being picked (replay start, Pine Logs) has the drag to itself: Demonstration draws nothing
  while it is on, and lets go of a stroke begun before it.
- The marks go when the cursor changes or the card shows another market (`abort_drawing`).
- A translucent stroke is one path, so where it crosses itself during the fade the overlap is a
  little darker; the colour is the app's orange (**ours**).

## Values tooltip on long press

The switch at the foot of the cursor menu (`DrawingPrefs::values_tooltip`, in `drawing.prefs`, on by
default). `ui/chart/card/values_tooltip.rs`.

- **Long press.** The primary button goes down on the plot (not on an axis, an overlay widget or a
  drawing), the pointer stays within 6 px (egui's click distance) for **0.5 s** (**ours**; under
  egui's 0.8 s long touch, so touch shows it first), and the tooltip shows. Moving first is an
  ordinary pan or drawing drag and never shows it. Works for the three cursors that leave the
  press alone (Cross, Dot, Arrow); never for Demonstration or the Eraser.
- **While it shows** it follows the pointer, the press is the tooltip's (the chart does not pan, a
  drawing is not dragged), and it is gone when the button is let go. It never shows while a drawing
  tool is armed: that press places anchors and drags out shapes. With a drawing under the press
  the press is the drawing's.
- **Content, all from what the frame already computed** (`CrosshairBars`; nothing is recomputed):
  the time, the readout's rows (open, high, low, close or the constructed bar's own, volume, the
  market's price on a constructed type, scripts' `plotcandle`/`plotbar`), the change from the
  previous close, the built-in indicators over the bars in view (EMA, Bollinger, RSI, Supertrend
  flip), and every script plot that asks for the data window (`display` has
  `DISPLAY_DATA_WINDOW`), on the chart and in its pane, read at the hovered bar through
  `series_index` (so replay-cut and constructed bars line up).
- It sits right and below the pointer, on the other side where there is no room, and stays inside
  the plot under the header. It is never bigger than that room: as many rows are listed as fit
  above the next pane, the rest are counted in a last row (`+N more`), and a long name is cut with an
  ellipsis, and a long value (a script's candle row, eight decimals) wraps within 60 % of the width,
  so the box is never wider than the plot. In a plot too short for a row, only the time
  shows; in one too short for that, nothing. Drawn with the crosshair, so a snapshot never has it.
- On a touch screen egui turns a long touch into a right-click. The drawing layer ignores that one
  (`DrawingFrame::long_press_values`) only while the tooltip has the press, so a press the tooltip
  never took (on a drawing, locked or not, with the option off, or under Demonstration or the Eraser)
  keeps its context menu. **Not tried on a touch screen.**

## Favorites

Every row of the cursor menu has the same star as the drawing tools' flyouts, and a starred cursor is
a button on the same floating favorites bar ([Drawing tools](drawing-tools.md#favorites)). Nothing
read documents stars on cursors (a screenshot of TradingView's menu shows one on the Cross row, and
a third-party widget's docs show one beside each cursor); that they join the one favorites list is
**our choice**, so there is no second list.

- **One list.** `drawing.favorites` holds tool names and, for a cursor, `cursor:` and its name
  (`cursor:Eraser`; `model::drawings::favorites::Favorite`). An older build reads those as names it
  does not know, keeps them when it re-saves the list and leaves them out of its bar, so nothing is
  lost between builds. The list syncs; the bar's place stays on the device.
- **A button** is lit while its cursor is in force (no tool and no Zoom In holding the pointer).
  Pressing it picks the cursor exactly as the menu row does; pressing the lit one puts the cursor
  down, back to the Cross (as a lit tool's button puts the tool down). Starring changes nothing that
  is armed.
- The Cross row's star is as starrable as the others, so the bar can always take the pointer back.

## What TradingView documents, and what is our choice

Sources: the Help Center article on cursors (`support/solutions/43000703396`): Cross is the default,
Dot is "similar to the cross", Arrow is the "classic arrow without crosshair", and cursors "are not
actually drawing tools".

| | Cross | Dot | Arrow | Demonstration | Eraser |
| --- | --- | --- | --- | --- | --- |
| Lines | yes | no (**ours**: the docs say only "similar to the cross") | no (docs: "without crosshair") | no (**ours**) | no (**ours**) |
| Price and time tags | yes | yes (**ours**) | no (**inferred** from "without crosshair") | no (**ours**) | no (**ours**) |
| Box naming the bar | yes | yes (**ours**) | yes (**ours**) | no (**ours**) | no (**ours**) |
| Drag | pans | pans | pans | draws a fading stroke (docs) | sweeps to erase (**ours**) |
| Click | | | | Alt+click is a dot (docs) | erases (docs) |

Demonstration: the Help Center ("How to draw temporarily on the chart", 43000747626) and the
blog post announcing it say it works like the Highlighter, Alt/Option+click places dots, holding
the left button draws freely, and the marks disappear after a few seconds. **Not documented, so
not built**: a ring around the pointer, a click ripple, a trail on plain movement. Whether Alt must
stay held for a whole stroke is worded differently in the two pages; here only the dot needs it.

The Eraser: TradingView's pages say it removes drawings by clicking them (a masterclass page, seen in
search results only) and that Ctrl/Cmd+click on a Fibonacci level removes just that level (Help
Center 43000543443, **not done here**). Not found anywhere: dragging across several drawings,
locked or hidden drawings, undo, whether it stays armed. All of that is **our choice**: sweep, locked
protected, one undo step, stays armed until another cursor or Esc.
Nothing was compared with TradingView's own chart. Not found anywhere: the dot's size, whether Dot
keeps the lines, any cursor keyboard shortcut (none is bound), and whether TradingView remembers the
cursor between sessions (ours does).

## Known differences and unverified

- The Dot's size and ring are our design.
- The tile's click opens the menu; it no longer disarms a drawing tool by itself (picking a cursor,
  Esc, or arming another tool still does).
- No pointer ring, ripple or trail: nothing documents them. The fade timings and the colour are ours.
- The tooltip: TradingView documents the hold-to-show behaviour with a switch in the cursor menu, and
  (Charting Library 3.0) OHLC and change that follow the pointer; the **0.5 s**, the position, what
  else it lists and the dismissal on release are ours. The menu label "Values tooltip on long
  press" was not found in any source read.
- DegenView has no drawing groups, so there is nothing to erase together. Ctrl/Cmd+click on one
  Fibonacci level is not implemented.
- The inverted price scale does not exist in DegenView, so no cursor handles one.
- Not driven by touch, and not seen on Windows or Linux.
