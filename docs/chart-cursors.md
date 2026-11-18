# Chart cursors

The cursor menu on the tool rail's first tile: Cross, Dot, Arrow and the Eraser (Demonstration
follows in a later change). There is no Magic cursor and none is planned.

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
  the other options. Only a resting cursor (`CursorMode::persists`: Cross, Dot, Arrow) is stored: the
  Eraser is a mode of the moment, so a restart (or another Mac) never opens in a mode that deletes.
  Esc (after Zoom In and an armed tool) puts the Eraser down, back to the resting cursor.
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

## What TradingView documents, and what is our choice

Sources: the Help Center article on cursors (`support/solutions/43000703396`): Cross is the default,
Dot is "similar to the cross", Arrow is the "classic arrow without crosshair", and cursors "are not
actually drawing tools".

| | Cross | Dot | Arrow | Eraser |
| --- | --- | --- | --- | --- |
| Lines | yes | no (**ours**: the docs say only "similar to the cross") | no (docs: "without crosshair") | no (**ours**) |
| Price and time tags | yes | yes (**ours**) | no (**inferred** from "without crosshair") | no (**ours**) |
| Box naming the bar | yes | yes (**ours**) | yes (**ours**) | no (**ours**) |
| Drag | pans | pans | pans | sweeps to erase (**ours**) |

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
- DegenView has no drawing groups, so there is nothing to erase together. Ctrl/Cmd+click on one
  Fibonacci level is not implemented.
- The inverted price scale does not exist in DegenView, so no cursor handles one.
- Not driven by touch, and not seen on Windows or Linux.
