# Fibonacci and Gann tools

Fifteen drawing tools in the **Fibonacci and Gann** flyout off the strip, in the two sections
TradingView gives them: FIBONACCI (eleven) and GANN (four). They are the same kind of thing as every
other drawing (placed, previewed, selected, moved, reshaped, restyled, hidden by timeframe, locked,
undone, saved, synced, listed in the Object tree, drawn in snapshots): a tool is a row of
`Tool::spec`, a `Geometry` variant and a projection. This page is what is specific to them. The
retracement used to be a drawing of its own with seven fixed levels; it is now the first of these
(`Tool::FibRetracement`, MCP name `fib_retracement`), and a retracement an older build saved as that
variant is no longer read.

Provenance, as in [Drawing tools](drawing-tools.md): **doc** is stated by a TradingView help page,
**lib** by the Charting Library's `<Name>LineToolOverrides` pages (defaults), and **inferred** is
chosen here because nothing found says. Nothing was compared pixel for pixel against TradingView (no
access), so the visual checklist at the end is a manual test matrix, not a result.

Sources: TradingView Help Center articles for each tool (ids 43000518158 retracement, 43000518137
extension, 43000791610 channel, 43000518155 time zone, 43000518156 speed fan, 43000518136 trend-based
time, 43000518159 circles, 43000791611 spiral, 43000518157 speed arcs, 43000518153 wedge,
43000518143 pitchfan, 43000518152 Gann box, 43000791675 Gann square fixed, 43000518149 Gann square,
43000518151 Gann fan) and the Charting Library override pages `linetool*`.

## Implementation matrix

| Tool | Anchors (placement order) | Construction | Persisted | Tests |
|---|---|---|---|---|
| Fib retracement | 2: start, end (**doc**) | level 0 at the end, 1 at the start; price = end + (start − end) × ratio | `Geometry::Fib{FibRetracement}` + `FibStyle` | `model::fib`, `fib_tests` |
| Trend-based fib extension | 3: start of move, end of move, end of retracement (**doc**) | price = P3 + (P2 − P1) × ratio | same | same |
| Fib channel | 3: P1, P2 base line, P3 sets the 0 to 1 width (**doc**) | level n = base line moved by n × (P3's vertical distance from the base) | same | same |
| Fib time zone | 2: base interval P1 to P2 (**doc**) | lines at bar P1 + multiple × (P2 − P1), multiples 0, 1, 2, 3, 5, 8, … 89 (**lib**); counted in bars, never calendar time | same | same |
| Fib speed resistance fan | 2 (**doc**) | rays from P1 through the points at each ratio of the box P1-P2 on its far side and top; grid of the same ratios | same | same |
| Trend-based fib time | 3 (**doc**) | lines at bar P3 + ratio × (P2 − P1) (**inferred**: the help page does not say how P3 is used) | same | same |
| Fib circles | 2 (**doc**) | centre the midpoint of P1-P2; ratio r is the ellipse through both anchors scaled by r (**inferred**); Shift draws the trend line at 45° so the circles are round (**doc**) | same | same |
| Fib spiral | 2: centre, a point on the curve (**doc**) | golden spiral r = r0 × φ^(2θ/π) in screen space, clockwise unless flagged (**doc**) | same | same |
| Fib speed resistance arcs | 2 (**doc**) | arcs about P2 at ratio × the trend line's length, the half facing P1 unless "full circles" (**doc**) | same | same |
| Fib wedge | 3 (**inferred**: the help page says two points, but a sector needs a third): apex, radius arm, angle arm | arcs about the apex at ratio × the first arm, between the two arms; both arms drawn | same | same |
| Pitchfan | 3: P1 apex, P2, P3 (**doc**) | rays from P1 through the median's midpoint ± ratio × half the P2-P3 span; median is the ray through the midpoint | same | same |
| Gann box | 2 (**doc**) | rectangle P1-P2 divided at price levels and time levels; optional corner angles | `Geometry::Gann{GannBox}` | `model::gann`, `gann_tests` |
| Gann square fixed | 2 (**doc**) | levels, fan lines and arcs in a square the user sees as square when placing (**inferred**) | same | same |
| Gann square | 2 (**doc**) | the same in a free rectangle; optional price-per-bar override and a ranges readout (**doc**) | same | same |
| Gann fan | 2: origin, a point on the 1×1 line (**doc**) | rays from P1 at ratio × the slope of P1-P2 in screen space; 1×1 passes P2 | same | same |

## Defaults (lib)

Levels are 2 px, background fill 20 % (transparency 80), labels 12 pt.

- **Retracement, extension and channel**, 24 slots: 0 `#787B86`, 0.236 `#F23645`, 0.382 `#FF9800`,
  0.5 `#4CAF50`, 0.618 `#089981`, 0.786 `#00BCD4`, 1 `#787B86`, 1.618 `#2962FF`, 2.618 `#F23645`,
  3.618 `#9C27B0`, 4.236 `#E91E63` (all on), then off: 1.272, 1.414, 2.272, 2.414, 2, 3, 3.272,
  3.414, 4, 4.272, 4.414, 4.618, 4.764. Trend line `#787B86` dashed. Labels left, bottom (channel:
  left, middle).
- **Time zone**, 11: 0 (`#787B86`), 1, 2, 3, 5, 8, 13, 21, 34, 55, 89 (`#2962FF`); no fill; labels
  right, bottom; trend line `#808080` dashed 1 px.
- **Speed fan**: 7 price and 7 time levels 0, 0.25, 0.382, 0.5, 0.618, 0.75, 1; grid
  `rgba(21,56,153,.8)` 1 px; labels on all four sides.
- **Trend-based time**, 11: 0, 0.382, 0.5 (off), 0.618, 1, 1.382, 1.618, 2, 2.382, 2.618, 3.
- **Circles and speed arcs**, 11: 0.236, 0.382, 0.5, 0.618, 0.786, 1, 1.618, 2.618, 3.618, 4.236,
  4.618. **Wedge**: the same with 1.618 and above off, trend line solid `#808080`.
- **Spiral**: `#00BCD4`, 1 px. **Pitchfan**: median `#F23645`; 0.25, 0.382, 0.5, 0.618, 0.75, 1,
  1.5, 1.75, 2 with 0.5 and 1 on.
- **Gann fan**, 9: 1×8, 1×4, 1×3, 1×2, 1×1, 2×1, 3×1, 4×1, 8×1, all on.
- **Gann box**: price and time levels as the speed fan; angles off.
- **Gann square and fixed**: 6 levels (0, 0.25, 0.382, 0.5, 0.618, 1 — **inferred** from the colours
  the library gives them), 11 fan lines (8×1, 5×1, 4×1, 3×1 off; 2×1, 1×1, 1×2 on; 1×3, 1×4, 1×5,
  1×8 off), 11 arcs (1, 1, 1.5, 2, 2, 3, 3, 4, 4, 5, 5 sides; **inferred** that the second number
  picks the corner the arc is centred on).

## Placement and the tile

The **Fibonacci and Gann** tile is third on the strip, where the Fib tile was. It shows the last tool
used from its menu and opens FIBONACCI / GANN from its corner, a right-click or the Right arrow
(`Menu::Fib`, remembered in `DrawingPrefs.fib_tool`). Alt+F arms the retracement (**inferred**: the
TradingView default). Click-click or press-drag places the first two anchors, a click each the rest
(`Placement::Points(n)`); Esc, a tool switch or a right-click abandons a drawing and nothing is saved
or synced until the last anchor is placed. The preview is the same projection as the finished drawing.
Shift while placing the circles holds their trend line at 45 degrees, so they are round (**doc**). A
fixed Gann square is held square on the screen whenever it is placed or a corner is moved, with or
without Shift.

## Settings

Every tool's dialog has Style, Levels, Coordinates and Visibility (the spiral has no Levels); the
floating toolbar has the trend line's colour, thickness and style, the background and the usual
settings, lock, hide, duplicate and delete. A control a tool does not honour is not offered
(`FibKind::features` and `GannKind::features` are the one list the dialog reads).

- **Levels** (one editor for every list, `fib_pages.rs`): a switch, the ratio, the colour, a chip for
  thickness and line style (with Move up and Move down), a remove button, Add level (the list's
  capacity is 24, or 12 for a second or third list) and Restore defaults. Ratios are clamped to ±100,
  a repeated ratio is dropped, and the lists are fixed arrays so `DrawingStyle` stays `Copy`.
- **Style**: use one colour (every level in the line colour, **doc**), trend line on or off, reverse,
  the log-scale levels switch, full circles, counterclockwise, the second background, the grid (a
  box's angle rays), price per bar and the ranges readout of a standard square.
- **Labels**: the level value, percentages (`61.8%`), the price, where along the line and on which side
  of it the text sits, the four sides of a fan or a box, and the font size. Prices are real prices in
  every price-scale mode.
- **Background**: the usual fill switch and colour; a fill is the colour of the level it ends at, at the
  fill colour's opacity (20 % by default). A fill is a **backdrop**: seen, never a place to click the
  drawing (a time zone's strips cover the whole chart).

Not offered, and not done: TradingView lets a level's text be typed on the chart, and an alert on a
retracement's levels ("Add alert on Fib retracement"). Neither is here.

## How each tool is drawn

Levels are worked from stored prices and bar indexes, never from pixels, and mapped to the screen only
at the end, so a retracement is the same on a Regular, Percent, Indexed to 100 or Logarithmic axis.
Where TradingView builds a figure in screen space (circles, arcs, the spiral, the wedge, the Gann
angles) it is built there, from the anchors' screen positions.

| Tool | Construction here |
|---|---|
| Retracement | level 0 on the end, 1 on the start (**inferred**: the older Fib had 0 on the start). Price = end + (start − end) × ratio. On a log chart with "levels based on log scale" the ratio divides the move of the logarithm (0.5 is the geometric mean); without it, the difference, even on a log chart (**doc**: the switch is only for log charts). Lines span the anchors, or run to the plot's edges when extended. |
| Extension | price = P3 + (P2 − P1) × ratio, in the logarithm likewise; reverse projects the move the other way. Lines span all three anchors. |
| Channel | the base line P1-P2; the offset is P3's vertical distance from it on screen (as the parallel channel), so every level is parallel and spaced by the third anchor, on any scale. Level 1 passes through P3. |
| Time zone | lines at bar P1 + multiple × (P2 − P1), the multiples 0, 1, 2, 3, 5, 8 … 89, counted in the chart's own bar index (**doc**: anchors are bar numbers). A weekend with no bars is no distance; the future past the last bar extrapolates at the average interval; no interval draws no lines. At most one line per level. |
| Speed fan | rays from P1 through each ratio of the box P1-P2 on its far vertical side (price levels) and far horizontal side (time levels), plus a grid of the same ratios. Reverse opens the fan from P2. |
| Trend-based time | lines at bar P3 + ratio × (P2 − P1) (**inferred**: the help page does not say how P3 is used). |
| Circles | centre the middle of P1-P2; the ratio 1 ellipse has radii (\|dx\|, \|dy\|) × √2 / 2 and so passes through both anchors (**inferred**); under Shift they are circles. |
| Spiral | the golden spiral about P1 through P2, a quarter turn growing it by φ, clockwise unless flagged (**doc**), followed in to a quarter of a pixel and out past the plot. |
| Speed arcs | centre P2, radius ratio × \|P1P2\| in pixels, the half facing P1 unless "full circles" (**doc**). |
| Wedge | apex P1, first arm and radius to P2, second arm towards P3 (three anchors, **inferred**: the help page says two, a sector needs a third); arcs at ratios of the first arm between the arms, the short way. |
| Pitchfan | rays from P1 through the middle of P2-P3 (the median) and through the middle ± ratio × half the span, which are the pitchfork's levels with every ray meeting at the first pivot (it reuses `line_geometry::fork`). |

### Gann scaling

**A Gann angle is a ratio of price to time, never a screen angle** (`model/gann.rs`). The 1×1 passes
through the second anchor, whatever the chart's scales are; 2×1 climbs twice as fast and 1×2 half as
fast, on the screen positions of the two anchors, so a zoom or a different price scale moves the
anchors and the rays follow. 8×1 is eight units of price to one of time. The ratio a custom angle is
given is stored as a number (`ratio` = price ÷ time); its name is `8x1` for a ratio of 1 or more and
`1x8` for less.

- **Fan**: nine angles 1×8 … 8×1, all drawn, from the first anchor on to the plot's edge.
- **Box**: price levels at ratios of the price range and time levels at ratios of the bar range
  (seven each: 0, 0.25, 0.382, 0.5, 0.618, 0.75, 1), two fills, labels on the four sides, and off by
  default a set of corner rays (the two diagonals, and from each corner a ray to the middle of the
  sides it is not on). Reverse counts every ratio from the other corner.
- **Square**: a box of any shape from its first corner: six grid levels, eleven fan lines (2×1, 1×1
  and 1×2 drawn), eleven arcs (quarter ellipses of a number of sides, about the first corner or the
  far one), their fills, and optionally its ranges, the bars across, the price range and the price per
  bar. **Price per bar** sets how tall it is (as many bars across as the anchors are apart, and that
  many times the scale up); Shift while a corner moves keeps the ratio it had.
- **Square fixed**: the same, held square on the screen when it is placed and when a corner is edited,
  and without the readout and the scale. What stays fixed is that square: once placed it is two
  anchors like any drawing, so a zoom changes its shape and the price per bar its 1×1 means is the one
  it had when it was drawn. (**Inferred**: TradingView's two help pages differ only by the scale and
  the ranges readout.)

TradingView's Gann arcs are named by two numbers (`1x0`, `1x1`, `1.5x0`); the second is taken here to
be the corner the arc is about (0 the first, 1 the far one), shown as Near and Far in the editor
(**inferred**). Its six grid levels have no ratio in the library; 0, 0.25, 0.382, 0.5, 0.618 and 1 are
taken from the colours it gives them (**inferred**).

## What is stored

A Fibonacci tool is `Geometry::Fib { kind, points }` and a Gann tool `Geometry::Gann { kind, points }`
in the one drawing record (the Pattern and Wave precedent: the kind says how many anchors there are,
and `is_valid` checks it). The levels and switches are `DrawingStyle.fib: Option<FibStyle>`, left out of
every other drawing; `None` draws the tool's own defaults, so a drawing made by code or sent over MCP
with no levels still shows them. Nothing derived (a pixel, a radius, a ray) is stored.

- **Older builds**: a new tool is a new `Geometry` variant, so an older build keeps the drawing as
  `Unreadable` and syncs it byte for byte (no `DRAWING_FORMAT` change). A default style or a template
  saved for one of these tools makes `drawing.defaults` and the templates unreadable to an older
  build, as for every new tool ([iCloud Sync](icloud-sync.md#newer-versions)).
- **Sync**: no record kind of its own; the drawing record carries the levels. A test sends every tool,
  customised, through two Macs and checks one upload per edit.
- **Templates and defaults**: a tool's default style is its `FibStyle` too, so a template keeps the
  levels.
- **MCP**: the tools are listed read-only by name (`fib_retracement`, `fib_extension`, `fib_channel`,
  `fib_time_zone`, `fib_speed_fan`, `fib_trend_time`, `fib_circles`, `fib_spiral`, `fib_speed_arcs`,
  `fib_wedge`, `pitchfan`, `gann_box`, `gann_square_fixed`, `gann_square`, `gann_fan`) and deletable,
  not created. The retracement's name was `fib`.
- **Snapshots**: the live frame's painter, so levels, fills and labels are the same and handles are
  never in them; a test compares both frames for every tool.

## Performance

A drawing's screen shape is cached until its anchors, its style or the view change. Every curve is
sampled where it crosses the plot and to a quarter of a pixel (`ui::fib_geometry`), with a budget of
6,000 points and at most 512 starting pieces whatever the anchors are, and a fill is made of convex
pieces cut to the plot, so a circle a million pixels wide is as cheap and as smooth as one on screen. A
list has at most 24 levels, so a set of time lines or levels is at most that many lines.

## Tests

- `model::fib`, `fib_style`, `gann`: the level arithmetic (retracement, extension, logarithm, bar
  lines), the lists (add, remove, reorder, restore, clamp, repeat), the angle and arc names, the
  scale; every number worked by hand.
- `ui::fib_geometry`: a ring is one radius everywhere and flat to a quarter of a pixel, a circle far
  bigger than the plot is sampled only where it crosses, the spiral is φ wider a quarter turn on and
  bounded, the discs, annuli and fans have the right area and cover the right points.
- `drawing_layer::fib_tests` and `gann_tests`: each tool on a plot of 100 px a bar and 10 px a price
  step: levels, spans, fills, labels, reverse, extend, the log switch, bars not calendar time,
  picking, moving, the 1×1 through the second anchor at several zooms and price scales, the fixed
  square, the ratio held under Shift.
- The every-tool loops over `Tool::GROUPED` (placement is one undo step, select, move, delete, every
  handle reshapes, extreme and broken anchors, log, percent and indexed scales, templates, save and
  reopen, snapshots).
- Sync (two Macs), the saved view round trip, MCP names, the tile's menu order, the dialog's pages.

## Not done / unverified

- Nothing was run in a real window: the hand-run galleries draw every tool, the menu, the dialog pages
  and a selection from the app's own renderer, which is not the app and not TradingView.
- No pixel comparison with TradingView. The defaults are the Charting Library's; the choices marked
  **inferred** above are mine.
- Typed text on a level and alerts on the levels are not done.
- A level's label can overlap another where levels are close: labels give way along the way ratios
  grow, as far as five times, and then stay put.
- Per-timeframe visibility, lock, hide, duplicate and delete are the generic ones; no tool has its own.
- A retracement an earlier build saved is not read (it stays in the Object tree as an unreadable row,
  deletable).

## Manual visual test matrix

Compare against TradingView in both themes, at 1× and 2× scale.

| Check | Where |
|---|---|
| Default levels, colours, fills and labels of each tool | all fifteen |
| Label placement left, centre, right and above, on, under the line | retracement, channel |
| Log chart with and without the log switch | retracement, extension |
| Circles round under Shift; ring smoothness at huge zoom | circles, speed arcs |
| The 1×1 through its anchor after a vertical zoom and a price-scale change | Gann fan, box, square |
| Fixed square staying square while a corner is dragged | Gann square fixed |
| Time lines across a weekend, into the future, and in replay | time zone, trend-based time |
| The level editor with 24 rows, add, remove, reorder, restore | retracement, Gann square |
| The tile's menu in a short window, its shortcut column, keyboard | Fibonacci and Gann tile |
| Snapshot against the live chart | all fifteen |
