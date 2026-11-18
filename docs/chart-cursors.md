# Chart cursors

The cursor menu on the tool rail's first tile: Cross, Dot and Arrow (Demonstration and the Eraser
follow in later changes). There is no Magic cursor and none is planned.

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
  the other options.
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

## What TradingView documents, and what is our choice

Sources: the Help Center article on cursors (`support/solutions/43000703396`): Cross is the default,
Dot is "similar to the cross", Arrow is the "classic arrow without crosshair", and cursors "are not
actually drawing tools".

| | Cross | Dot | Arrow |
| --- | --- | --- | --- |
| Lines | yes | no (**ours**: the docs say only "similar to the cross") | no (docs: "without crosshair") |
| Price and time tags | yes | yes (**ours**) | no (**inferred** from "without crosshair") |
| Box naming the bar | yes | yes (**ours**) | yes (**ours**) |
| Drag | pans | pans | pans |

Nothing was compared with TradingView's own chart. Not found anywhere: the dot's size, whether Dot
keeps the lines, any cursor keyboard shortcut (none is bound), and whether TradingView remembers the
cursor between sessions (ours does).

## Known differences and unverified

- The Dot's size and ring are our design.
- The tile's click opens the menu; it no longer disarms a drawing tool by itself (picking a cursor,
  Esc, or arming another tool still does).
- The inverted price scale does not exist in DegenView, so no cursor handles one.
- Not driven by touch, and not seen on Windows or Linux.
