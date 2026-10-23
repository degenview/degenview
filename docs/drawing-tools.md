# Drawing tools

Thirty-three drawing tools beyond Fibonacci retracement and the ruler, in two flyout menus off the
strip: lines, channels and pitchforks (the trend line and sixteen more), and brushes, arrows and
shapes. They have the lifecycle TradingView users expect (place, preview, select, move, reshape,
restyle, label, edit coordinates, hide by timeframe, lock, undo, save). One interaction layer,
one style model and one geometry module serve all of them; a tool is a row in a table
(`Tool::spec`), not a code path.

The text, note and content tools (Text, Note, Price note, Pin, Table, Callout, Comment, Price label,
Signpost, Flag mark, Image, Post) are in a third flyout and are documented in
[Text, note and content tools](annotation-tools.md); they share everything below except what that page
says. The chart patterns, Elliott waves and cycles (XABCD, Cypher, Head and shoulders, ABCD, Triangle
pattern, Three drives; five Elliott waves; Cyclic lines, Time cycles, Sine line) are in a fourth, the
Patterns tile, and are documented in [Chart patterns, Elliott waves and cycles](pattern-tools.md). The forecasting,
volume and measurement tools (Long and Short position, Position forecast, Bars pattern, Ghost feed,
Sector; Anchored VWAP and two volume profiles; Price, Date and Date and price range) are in a fifth,
the Forecast tile, documented in [Forecasting, volume and measurement tools](forecast-tools.md).

TradingView's own documentation and its Charting Library defaults are the reference. What they
state is marked **doc** or **lib**; what they do not state, and was chosen here, is marked
**inferred**. Nothing was compared pixel for pixel against TradingView (no access), so the visual
parity checklist at the end is a manual test matrix, not a result.

## Lines, channels and pitchforks

The Lines tile (second on the strip) shows the last tool used from its menu and opens
LINES / CHANNELS / PITCHFORKS from its corner, a right-click or the Right arrow. Fibonacci keeps
its own tile and is in neither menu. Alt (Option on a Mac) with **T**, **H**, **V** and **C** arms
the trend line, horizontal line, vertical line and crossline (**doc**, community posts); **J** arms
the horizontal ray (TradingView's default, which no help page states: **inferred**). Hotkeys leave
a focused text field alone.

| Tool | Anchors / placement | What is drawn | Default (lib unless noted) |
|---|---|---|---|
| Trend line | 2 | segment, extend left/right (past the first / second anchor) | DegenView's own `#0A84FF` 1.5 px |
| Ray | 2 | from the first anchor through the second, on to the plot's edge | `#2962FF` 2 px, extend right on |
| Info line | 2 | trend line and a statistics box (price range, %, bars, time, distance, angle) | stats on and always shown |
| Extended line | 2 | through both anchors, no end | extend both on |
| Trend angle | 2 | trend line, a level reference, an arc and the angle in degrees | angle **inferred**: the line's rise to the right on screen, positive up |
| Horizontal line | 1 | across the plot at a price; price tag on the axis | price tag on, text above |
| Horizontal ray | 1 | from the origin to the right edge; price tag | price tag on |
| Vertical line | 1 | across the plot at a time; time tag on the axis | time tag on, text vertical |
| Crossline | 1 | both of the above through one point | both tags on |
| Parallel channel | 3: first edge (2), then a point on the second | second edge parallel at a price offset, middle line, 4 optional levels, fill | `#2962FF`, middle line dashed, fill 20 % |
| Regression trend | 2 (the bar range) | least squares of a price over the bars, bands at deviations, Pearson's R | source close, +2 / -2, base `rgba(242,54,69,.3)` dashed |
| Flat top/bottom | 3: sloped edge (2), then the level | sloped edge and a level one at the third click's price over the same times, fill | `#FF9800`, fill 20 % |
| Disjoint channel | 3: first edge (2), then the start of the second | two parallel edges with their own start and end times, fill | `#089981`, fill 20 % |
| Pitchfork | 3 | median from P1 through the middle of P2-P3; levels parallel to it | median `#F23645`; levels 0.5 and 1 on of nine, fill 20 % |
| Schiff | 3 | as above, from P1's time at the price halfway to P2 | (as Pitchfork) |
| Modified Schiff | 3 | as above, from the middle of P1-P2 in time and price | (as Pitchfork) |
| Inside | 3 | median from the middle of P1-P2 to P3; levels measured across P1-P2 | (as Pitchfork) |

What is **doc**: the anchor counts and click orders, the median constructions (TradingView's Auto
Pitchfork page), the inputs of the regression trend, the settings each tool lists. What is
**inferred**: how far an unextended pitchfork runs (`FORK_LENGTH`, two handle lengths), that the
Inside fork's levels are measured across P1-P2, that a disjoint channel's edges stay parallel, that
a flat channel's level edge spans the sloped edge's times, the population divisor of the
regression's deviation, fitting in logs on a logarithmic scale, and the angle's sign.

### Geometry

All of it is worked in screen space from the projected anchors (`ui::line_geometry`, pure, with
hand-worked fixtures). Lines are straight on every price scale, and midpoints, parallels and
ratios survive the chart's zoom and offset, so a fork or a channel keeps its shape when the chart
moves. Nothing derived is stored: a drawing holds its anchors and its style.

- **Lines with no end** are clipped to the plot exactly (Liang-Barsky on the line's parameter in
  `f64`, the range left open on an unbounded side), never drawn from a far-away coordinate; one
  that misses the plot is not built, so it cannot be picked. A zero-length line has no direction
  and draws nothing but its handles.
- **Parallel channel**: the second edge keeps the first edge's times and moves by a vertical price
  offset (a perpendicular one for a vertical first edge). Dragging an end of the first edge keeps
  the offset; the second edge's handle changes only the offset.
- **Flat channel**: the third anchor's time is kept at the middle of the first two, so only its
  price is its own. **Disjoint channel**: four anchors, the fourth read only for its time.
- **Pitchforks** (`line_geometry::fork`): the four constructions differ only in where the median
  starts and ends; level `r` is the median shifted by `r` half-bases on both sides, so level 1
  passes through P2 and P3 (P1 and P2 for Inside). The variant is part of the geometry, so the
  dialog switches it without redrawing and the pivots are kept.
- **Regression** (`model::regression`): ordinary least squares of the chosen source over the bars
  the two anchor times fall on, both ends included, with the bar position as x (so gaps follow
  what the chart shows). It reads the bars the chart draws, so a constructed chart type and a
  replay cut work and nothing after the replay cursor is read. The fit is cached in the cached
  screen shape; the cache key now includes the newest bar's open/high/low/close, so a live tick
  refits. Anchor prices are never read.
- **Text** along a line is turned to follow it (never upside down) by `TextShape`; its
  horizontal alignment places it along the visible stretch and its vertical alignment above, on or
  below. A vertical line's text reads up the line or across it (`TextOrientation`).

### Text on lines, edited in place

A selected, unlocked line or channel that can carry text shows a faint "Add text" at the place the
text goes. A click on it, or on text it has, opens an editor there (an egui text field, so
selection, the clipboard and input methods are egui's). Return or leaving the field keeps the
text, Shift+Return adds a line, Escape drops the edit and is spent on it. The edit is one
`Replace` when it ends: one undo step, nothing written while typing, and the app's shortcuts
(Delete, undo, the tool hotkeys) leave the keys alone while the field has the keyboard. The
toolbar's Text button and the right-click menu open it too, and the dialog's Text tab edits the
same text.

### Storage and sync

- The app has not shipped with the older forms, so nothing is kept for them: the trend line is a
  `Shape` with a `trend_line` geometry like every other line (it was a variant of its own), and a
  row saved by an earlier development build may be unreadable now.
- Every style field the new tools added is left out of a stored drawing while it holds its default
  (`skip_serializing_if`), so a rectangle does not carry nine pitchfork levels around in every
  layout and synced record.
- Drawings sync as part of saved layouts, one record each, as JSON: the new tools needed no change
  to the sync engine. Tests run all seventeen through two Macs on `MockCloud`, edit and delete
  them, and round-trip a trend line gaining and losing text. Real CloudKit and two real Macs were
  not exercised.
- A row this version cannot read (a tool from a newer build) is kept and written back untouched.
  `merge_remote_drawings` used to treat every such drawing as one drawing (their `id()` is 0); it
  now matches them by the id they were stored with.

## Trend line style

The trend line carries a `DrawingStyle` like the other lines: colour, width, pattern, extend left
and right, arrow ends, price labels, statistics and text. Its default is DegenView's own blue
1.5 px (`DrawingStyle::trend_line()`). It is projected by the same code as the other lines
(`ui/drawing_layer/line_shapes.rs`); the painter prolongs it to the plot edge and the hit test
follows the prolongation, and its ends can be grabbed without selecting it first. The properties
dialog edits it through the tool's capabilities. Drawings an MCP client makes use the same model
(see `docs/architecture/mcp.md`).

## Tools

| Tool | Anchors | Placement | Finish | Style offered | Doc / default |
|---|---|---|---|---|---|
| Brush | many | press, drag, release | release | colour, thickness | gesture inferred; orange kept (the app's own default) |
| Highlighter | many | press, drag, release | release | colour+opacity, thickness | doc gesture; lib `rgba(242,54,69,0.2)`, width 20 |
| Arrow marker | 2 | click, move, click (or drag) | 2nd anchor | colour, thickness, text | doc; lib `#1E53E5`, text 16 bold |
| Arrow | 2 | click-click or drag | 2nd anchor | colour, thickness, pattern, start and end heads, middle point, price labels, statistics, text | lib `#2962FF`, end head |
| Arrow mark up | 1 | click | the click | colour, text | doc fixed size; lib `#089981` |
| Arrow mark down | 1 | click | the click | colour, text | doc fixed size; lib `#CC2F3C` |
| Rectangle | 2 | click-click or drag, Shift = square | 2nd anchor | border, pattern, fill, middle line, extend left/right, text | doc; lib `#9c27b0`, fill 20 % |
| Rotated rectangle | 3 | edge start, edge end, width point | 3rd click | border, pattern, fill | doc; lib `#4caf50` |
| Path | any | click each | double-click, Return, Esc (≥ 2 points), right-click | colour, thickness, pattern, start and end heads | doc; lib `#2962FF`, end head |
| Circle | 2 | centre, then a radius point | 2nd click | border, pattern, fill, text | doc; lib `#FF9800` |
| Ellipse | 2 | bounding corners, Shift = circle | 2nd anchor | border, pattern, fill, text | doc Shift; corners inferred; lib `#F23645` |
| Polyline | any | click each; click the first point to close | double-click, Return, Esc, right-click (open) | border, pattern, fill (closed) | doc; lib `#00bcd4` |
| Triangle | 3 | 3 clicks | 3rd click | border, pattern, fill | doc anchors; lib `#089981` |
| Arc | 3 | start, end, curvature point | 3rd click | border, pattern, fill | inferred; lib `#e91e63` |
| Curve | 3 | start, end, control | 3rd click | line, pattern, heads, extend, fill | doc; lib `#2962FF`, no fill |
| Double curve | 4 | start, end, control 1, control 2 | 4th click | as Curve | doc; lib `#673ab7`, no fill |

Common to all: a floating toolbar and a properties dialog (Style, Text where the tool has text,
Coordinates, Visibility), a right-click menu, drag to move, handles to reshape, lock, hide,
duplicate, copy and paste style, stacking order, templates and a default style per tool.

## Geometry

Anchors are always market coordinates (time in milliseconds, price). Everything that is not a
coordinate is derived each frame, so a drawing follows pan, zoom, resize, a price-scale change and
a theme switch.

- **Rectangle**: opposite corners; stays axis-aligned. Extension continues to the plot edge in
  pixels computed per frame; the anchors never change.
- **Rotated rectangle**: two anchors for an edge and a third whose perpendicular distance sets the
  width. The other two corners are derived on screen, so it can never stop being a rectangle.
  Perpendicularity is in screen space, so the angle on screen follows the scales.
- **Circle**: centre and a point on the circumference. The radius is the on-screen distance, so it
  stays round when the axes are scaled (TradingView says the same of its circle).
- **Ellipse**: bounding-box corners; Shift squares the box during placement.
- **Arc**: the circular arc through three points. Collinear, coincident or enormous circles fall
  back to the straight chord.
- **Curve** and **double curve**: quadratic and cubic Béziers whose control points are anchors,
  flattened to within 0.25 px (at most 256 segments). A new curve is straight until its control
  is placed.
- **Arrow marks**: a fixed 20 × 26 px glyph whose tip sits 4 px from the anchor; never stretched
  by the price scale.
- **Polygons**: filled with the **even-odd** rule, exactly, by horizontal bands split where edges
  cross. Concave and self-intersecting polylines fill and hit-test the same way; a pentagram has a
  hole.
- **Freehand strokes** are simplified (Ramer–Douglas–Peucker, 0.4 px) when the stroke ends, keeping
  the sampled anchors rather than a projection round trip. Display smooths them (two Chaikin
  passes).

All of it is pure functions on screen points in `ui/drawing_geometry.rs`, tested without a window.

## Highlighter

A translucent stroke must have one alpha however the path crosses itself or bends tighter than its
own width, which a mesh of triangles cannot promise. Translucent freehand strokes are rasterized
into a coverage map on the physical pixel grid (a union by `max`, so overlap never adds up) and
painted as one image; edges are antialiased at any scale. Opaque strokes are a single mesh with
round caps and joins. Two separate strokes drawn over each other do darken, as on TradingView.

Strokes too big to hold as a map (over 12 M pixels or 60 M pixel visits) fall back to the mesh.

## Interaction

One controller (`ui/drawing_layer/`). A tool's `Placement` decides how anchors arrive:

- `Freehand`: drag. A click leaves a dot.
- `Single`: one click.
- `Points(n)`: a drag places the first two anchors, clicks place the rest; click-click works too.
- `Open`: a click adds a vertex; a double-click, Return, Esc (with at least two vertices),
  or a right-click finishes; Backspace takes the last vertex back; a click near the first vertex of
  a polyline closes it. At most 5,000 vertices.

Esc, switching tool, losing window focus, or the card going unseen (another tab) abandons a draft
without a command, so a cancelled drawing is never an undo entry. A press and release within two
pixels of each other makes no shape. Only the primary button draws and pans.

Selection hit-tests the real geometry: strokes, arrow glyphs, curves, and the interior of filled
shapes. A selected drawing's handles win over everything beneath them. Shift-click builds a
selection, which moves, locks, hides, duplicates and deletes together as one step.

Each shape has the handles its geometry needs (eight on a rectangle or ellipse, a radius handle on
a circle, three on a rotated rectangle with the width handle on the far edge, vertices on paths,
hollow control points on curves).

## Magnet and Keep drawing

The strip's magnet cycles Off → Weak → Strong. Strong snaps an anchor to the open, high, low or
close of the nearest bar; Weak only within 20 px. Holding ⌘/Ctrl turns it on when it is off and off
when it is on (**doc**). It acts on placement and on handle drags, never on freehand samples or on
moving a whole drawing, and it reads the constructed bars the chart draws (Renko bricks…) cut at
the replay cursor, so it cannot reach a bar replay has not shown.

Keep drawing (on by default, which is how the tools always behaved here) stays on the tool after
a drawing; off returns to the crosshair. Magnet, keep drawing and the grouped tile's tool persist.

## Styles, text, templates

`DrawingStyle` (`model/drawing_style.rs`) holds stroke, fill, text style, extension, middle line,
heads, middle point, price labels and statistics, every field with a serde default and
`sanitized()` on every write. `Tool::spec().caps` lists what a tool offers; the toolbar and the
dialog read it, so a control a tool cannot honour is never shown.

Templates (`model/drawing_template.rs`, `store/drawing_template.rs`) save a tool's style, never its
coordinates, text or visibility, and apply to the same tool only. A tool's default style is kept in
the `drawing.defaults` setting. They are separate from chart appearance templates and share only
the naming rules. Like the chart templates, the store re-reads the table before every write and
refuses to write over a row it cannot decode.

The properties dialog follows the chart settings pages: every card of a page sits 8 px apart,
a page with more than one group of controls titles each group (Line, Arrowheads, Shape, Extend,
Details, Statistics; Text, Font, Alignment), and the right-hand inputs share one 24 px height and
a 130 px outer width. A heading restores the card spacing it tightens for its subtitle
(`Page::section`), a card without a hint is not given an empty one (`toggle_row`), and the page
keeps the height of the tallest tab seen so switching tabs does not resize the dialog.

## Arrow statistics

Price range, percent change, bars, time and distance reuse the ruler's formatting
(`RulerReadout`, the chart's price format). **Distance** is the signed number of bars from start to
end (TradingView: "distance in periods"). **Angle** is the arrow's direction as drawn on screen;
TradingView does not document its formula, so this is inferred. Pips are not offered: no market
here has a pip convention to follow.

## Persistence and undo

Drawings are saved with their chart in the existing session and saved layouts; new fields
default. A chart's drawings decode one by one
(`model::drawings::lenient`): a row this version cannot read, such as a tool from a newer build,
stays in the list as `Drawing::Unreadable` and is written back byte for byte, instead of costing
the whole tab or layout. Geometry with a non-finite number or too few points is treated the same.

Undo is the workspace's existing stack of per-chart drawing lists. One batch of commands is one
step; a batch that changes nothing, or only the transient ruler, is none. Restyles that arrive in
a burst (dragging a colour picker) merge into one step (800 ms, same drawing). The properties
dialog edits a copy shown live and commits once.

A drawing edit does not mark a saved layout as unsaved, but an Autosave layout writes it
(`ChartWorkspace::drawings_dirty`); the session saves drawings every 10 s.

## Replay, synthetic charts, panes

Drawings project through the bars the chart draws: the constructed series on Renko, Kagi and the
others, and the replay-truncated candles during replay. Times between bars interpolate and times
beyond the data extrapolate at the average interval, so a drawing anchored after the replay cursor
stays where it is. Changing the chart type, the timeframe or the replay position never rewrites an
anchor; per-timeframe visibility is only a predicate on what is shown.

Drawings are main-pane only. Indicator panes are separate coordinate spaces and a drawing is never
reinterpreted as a pane value.

## Not done, approximate or unverified

- **Drawing alerts.** A horizontal line, horizontal ray or crossline offers "Add alert at this
  price" (toolbar and menu), which opens the alert editor on that price for the user to review; it
  saves nothing itself. A sloped line needs a time-dependent condition in the alert engine (which
  also runs in a separate agent process) and was left out rather than offering a button that does
  nothing.
- **The new tools against TradingView.** The geometry follows the documentation and the
  Charting Library defaults listed above; nothing was compared pixel for pixel. Alt+J, the
  unextended fork length, the Inside fork's levels, the disjoint channel's parallelism, the flat
  channel's time span, the regression's divisor and its logarithmic fit, and the trend angle's sign
  are inferred.
- **Trend angle** has no numeric Angle field (TradingView's Coordinates tab has one): setting it
  needs the live projection, which the dialog does not have. The angle is shown and follows the
  anchors.
- **Info line and trend angle** statistics reuse the arrow's (distance in periods, the on-screen
  angle); pips are not offered.
- **MCP** lists and deletes the new tools but cannot create or update them: its argument shape is
  two anchors, a colour and a width (`docs/mcp-remaining-work.md`).
- **Rotated text** is laid out by egui like any text and may look a little softer than level text
  on a low-density display.
- **Unreadable drawings** (a tool, or a field, from a newer build) are never drawn or picked, are listed
  in the Object tree as "From a newer version" (only Delete works), and are written back byte for byte.
  See [iCloud Sync](icloud-sync.md#newer-versions) for how newer data reaches an older build.
- **Pips**, as above.
- **Bar-index coordinate field.** The Coordinates tab edits price and a UTC date-time, the latter
  with the calendar pop-over and hour/minute stepper of `ui/date_picker.rs` (pure egui, so the
  same on every platform; no typed entry). A bar number is not offered.
- **Groups and an object tree**; a multi-selection is moved, locked, hidden and deleted together
  but not grouped.
- **Unverified against TradingView**: the Arc's three-point construction, the Curve control
  semantics (a true Bézier control, not a through-point), the angle formula, whether Return finishes
  a path, the floating toolbar's exact buttons, and every visual (line weights, handle sizes, fill
  alpha). Defaults come from the Charting Library's override pages, not from the product.
- **A single highlighter stroke** that crosses itself in mesh fallback (a stroke over 12 M pixels)
  can darken where it crosses. Separate strokes darken by design.
- **Locking and hiding all** act on every chart of the tab, with one undo step per chart because
  history is per chart. "Hide all" is a view toggle and is not saved.
- Market replacement (`switch_chart`) keeps a chart's old drawings, as before.

## Performance

Screen shapes are cached per drawing, keyed by the drawing and the projection, so panning the
pointer never re-flattens a curve; bounds reject drawings that are off screen before any painting
or hit-testing. Measured in an unoptimized build: 400 drawings (100 each of rectangles, arrows,
ellipses and curves, many off screen) take about 3 ms a frame; masking a 4,000-point stroke under
a second. `cargo test` runs these as tests with generous bounds.

## Manual visual test matrix

Compare against TradingView in both themes, at 1× and 2× scale. For each tool: default look after
placement, selected (handles, halo, toolbar), hovered, locked, with text, and under a log scale.

| Check | Where |
|---|---|
| Line weight and arrowhead size for Arrow, Path, Curve | 1.5 and 4 px widths |
| Highlighter blending where two strokes cross, and over candles | width 20 |
| Fill alpha over dark and light chart backgrounds | Rectangle, Ellipse, Arc |
| Arrow-mark size staying fixed while zooming the price axis | up and down |
| Handle size and hit area on a trackpad | rectangle corners |
| Grouped menu: clipping at the window edge, scrolling in a short window, keyboard | tile corner |
| Floating toolbar placement above, below and for a shape larger than the plot | big circle |
| Properties dialog pages and the template menu | every tool |
| Properties dialog: equal gaps after every heading, inputs aligned on the right edge, one hairline above the footer, no resize when switching tabs | trend line, Fib, Arrow, Rectangle, arrow mark, Brush |
| Lines menu: width, row alignment, shortcut column, scrolling in a short window, keyboard | Lines tile corner |
| Text along a line: sharpness at 1x and 2x, readable at every slope, above/on/below, a very short line | trend line, extended line, channel |
| "Add text" place and the editor: IME, clipboard, Return/Shift+Return/Escape, no shortcut firing | selected trend line |
| Option+T/H/J/V/C while typing in the editor and in a text field | macOS |
| Axis tags of a horizontal line and a vertical line, in a snapshot too | horizontal line, vertical line, crossline |
| Regression over a live bar, in replay, on a log scale and on a Renko chart | regression trend |
| Pitchfork levels, fills and the four variants on the same three pivots | all four forks |

The hand-run galleries (`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery -- --ignored`) write
the new tools, the menus, the dialog pages and the editor as PNGs from the app's own renderer, which
is a way to look at them without a window; they are not a comparison with TradingView.
