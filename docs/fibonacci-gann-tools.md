# Fibonacci and Gann tools

Fifteen drawing tools in the **Fibonacci and Gann** flyout off the strip, in the two sections
TradingView gives them: FIBONACCI (eleven) and GANN (four). They are the same kind of thing as every
other drawing (placed, previewed, selected, moved, reshaped, restyled, hidden by timeframe, locked,
undone, saved, synced, listed in the Object tree, drawn in snapshots): a tool is a row of
`Tool::spec`, a `Geometry` variant and a projection. This page is what is specific to them.

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

## Not done / unverified

Filled in as the work lands.
