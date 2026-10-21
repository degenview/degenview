# Drawing tools

Sixteen drawing tools beyond the trend line, Fibonacci retracement and ruler: brushes, arrows and
shapes, with the lifecycle TradingView users expect (place, preview, select, move, reshape,
restyle, label, edit coordinates, hide by timeframe, lock, undo, save). One interaction layer,
one style model and one geometry module serve all of them; a tool is a row in a table
(`Tool::spec`), not a code path.

TradingView's own documentation and its Charting Library defaults are the reference. What they
state is marked **doc** or **lib**; what they do not state, and was chosen here, is marked
**inferred**. Nothing was compared pixel for pixel against TradingView (no access), so the visual
parity checklist at the end is a manual test matrix, not a result.

## Trend line style

The trend line carries a `DrawingStyle` like the shapes: colour, width, pattern, and extend left
and right. A row saved before this had none and decodes to the blue 1.5 px segment it always was
(`DrawingStyle::trend_line()`); a line still in that look is written without a style, so stored
rows and saved layouts do not change. The painter prolongs it to the plot edge and the hit test
follows the prolongation. The properties dialog edits it through the tool's capabilities.
Drawings an MCP client makes use the same model (see `docs/architecture/mcp.md`).

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

Drawings are saved with their chart in the existing session and saved layouts. The four original
variants keep their serialized form; new fields default. A chart's drawings decode one by one
(`model::drawings::lenient`): a row this version cannot read, such as a tool from a newer build,
stays in the list as `Drawing::Unreadable` and is written back byte for byte, instead of costing
the whole tab or layout. Geometry with a non-finite number or too few points is treated the same.

Undo is the workspace's existing stack of per-chart drawing lists. One batch of commands is one
step; a batch that changes nothing, or only the transient ruler, is none. Restyles that arrive in
a burst (dragging a colour picker) merge into one step (800 ms, same drawing). The properties
dialog edits a copy shown live and commits once.

Drawings do not dirty a saved layout (they never did); the session saves them every 10 s.

## Replay, synthetic charts, panes

Drawings project through the bars the chart draws: the constructed series on Renko, Kagi and the
others, and the replay-truncated candles during replay. Times between bars interpolate and times
beyond the data extrapolate at the average interval, so a drawing anchored after the replay cursor
stays where it is. Changing the chart type, the timeframe or the replay position never rewrites an
anchor; per-timeframe visibility is only a predicate on what is shown.

Drawings are main-pane only. Indicator panes are separate coordinate spaces and a drawing is never
reinterpreted as a pane value.

## Not done, approximate or unverified

- **Drawing alerts.** A horizontal level could reuse the price-alert conditions, but a sloped line
  needs a time-dependent condition in the alert engine (which also runs in a separate agent
  process) and was left out rather than offering a button that does nothing.
- **Pips**, as above.
- **Bar-index coordinate field.** The Coordinates tab edits price and UTC date-time; a bar number
  is not offered.
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
