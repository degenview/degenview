# Chart patterns, Elliott waves and cycles

Fourteen drawing tools in a fourth flyout off the strip, the **Patterns** tile, in the three
sections TradingView gives them: CHART PATTERNS, ELLIOTT WAVES and CYCLES. They are the same kind of
thing as every other drawing (placed, previewed, selected, moved, reshaped, restyled, hidden by
timeframe, locked, undone, saved, synced, listed in the Object tree, drawn in snapshots): a tool is
a row of `Tool::spec`, a `Geometry` variant and a projection. This page is what is specific to them.

The tools are **manual drawings**. Nothing here scans the price for a pattern or a wave, scores one,
or moves a pivot the user placed. TradingView's Elliott help describes its rules (wave 2 never
retraces past the start of wave 1, wave 3 is never the shortest) for its *automatic indicator*, and
says nothing of the drawing tools enforcing them; so a wave that breaks them is drawn as placed.

Provenance, as in [Drawing tools](drawing-tools.md): **doc** is stated by a TradingView help page,
**lib** by the Charting Library's drawing overrides, **wiki** by TradingView's own wiki, and
**inferred** is chosen here because nothing found says. Nothing was compared pixel for pixel
against TradingView (no access), so the visual checklist at the end is a manual test matrix, not a
result.

## Tools

The Patterns tile shows the last tool used from its menu; the corner, a right-click or the Right
arrow open the menu. Click-click or press-drag places the first two pivots, a click each the rest
(the same `Placement::Points(n)` as every fixed-point tool). Backspace takes the last pivot back;
Esc, switching tool or a right-click abandons the drawing; nothing is saved or synced until the last
pivot is placed.

| Tool | Pivots (placement order) | What is drawn | Default (lib unless noted) |
|---|---|---|---|
| XABCD pattern | 5: X, A, B, C, D (**doc**) | the zigzag; triangles XAB and BCD filled; four dashed lines X-B, A-C, B-D, X-D each carrying a ratio | `#089981` (**inferred**: no override page), 2 px, fill 15 %, white 12 px labels |
| Cypher pattern | 5: X, A, B, C, D (**doc**) | the zigzag; triangles XAB and BCD filled; dashed X-B, X-C, X-D with ratios | `#2962FF`, 2 px, fill 15 % |
| Head and shoulders | 7 (**inferred**): start, left shoulder, left trough, head, right trough, right shoulder, end | the zigzag; the neckline through the two troughs from the first pivot's time to the last's; each shoulder and the head filled between the zigzag and the neckline; "Left Shoulder", "Head", "Right Shoulder" | `#089981`, 2 px, fill 15 % |
| ABCD pattern | 4: A, B, C, D (**doc**) | the zigzag; dashed A-C and B-D with ratios; no fill (the library has none) | `#089981`, 2 px |
| Triangle pattern | 4: A, B, C, D (**doc**: "four alternating highs and lows") | the zigzag, the lines A-C and B-D run on to their apex, the region between them filled | `#673AB7`, 2 px, fill 15 % |
| Three drives pattern | 6 (**inferred**): start, 1, A, 2, C, 3 | the zigzag filled as one polygon; dashed lines start-A, 1-2, A-C, 2-3 with ratios; labels 1, A, 2, C, 3 | `#673AB7`, 2 px, fill `rgba(149,40,204,.5)` |
| Elliott impulse wave (1·2·3·4·5) | 6: the start, then the end of each wave | the zigzag; a label at the end of each wave | `#3D85C6`, 2 px, degree Intermediate |
| Elliott correction wave (A·B·C) | 4 | as above | `#3D85C6` |
| Elliott triangle wave (A·B·C·D·E) | 6 | as above | `#FF9800` |
| Elliott double combo wave (W·X·Y) | 4 | as above | `#6AA84F` |
| Elliott triple combo wave (W·X·Y·X·Z) | 6 | as above; the second X is an X again, never renamed | `#6AA84F` |
| Cyclic lines | 2 | vertical lines at the distance between the two anchors, on and on; a dashed grey line joins the anchors (**lib**: `trendline`, on) | `#80CCDB`, 1 px |
| Time cycles | 2 | half circles over each stretch of that distance, filled | `#159980`, 2 px, fill `rgba(106,168,79,.5)` |
| Sine line | 2 | a sine wave through both anchors, which are consecutive extremes | `#159980`, 2 px |

The pivot counts of the Elliott tools follow from the names TradingView gives them (**inferred**: its
help says nothing of the number of clicks). Head and shoulders and Three drives are the two the
documentation cannot settle: its Head and shoulders page describes the formation and not the tool,
and its Three drives page says "five total points" (1, 2, 3, A, C) while its Coordinates tab lists
seven fields. Six pivots are used for Three drives (the five named and the point the first drive
starts from), and the seventh coordinate is not understood.

### Ratios

A ratio names its numerator and denominator legs (absolute price distances) and the line it is
printed on, the line from the start of the reference leg to the end of the leg the ratio is about.
The tools do not share one ratio function because they do not measure the same legs: TradingView's
XABCD page quotes a Gartley's D against XA and its Cypher page against XC.

| Pattern | Ratio | Numerator / denominator | Printed on | Source of the rule |
|---|---|---|---|---|
| XABCD | B | AB / XA | X-B | **doc** (Gartley: 0.618 of XA) |
| | C | BC / AB | A-C | **doc** (0.382 to 0.886 of AB) |
| | D | CD / BC | B-D | **doc** (1.272 to 1.618 of BC) |
| | D | AD / XA | X-D | **doc** (Gartley 0.786, Butterfly 1.272 to 1.618 of XA) |
| Cypher | B | AB / XA | X-B | **doc** (0.382 to 0.618 of XA) |
| | C | XC / XA | X-C | **doc** (an extension of XA, 1.13 to 1.414) |
| | D | CD / XC | X-D | **doc** (0.786 of the whole XC leg) |
| ABCD | C | BC / AB | A-C | **doc** |
| | D | CD / BC | B-D | **doc** (127.2 % or 161.8 % of BC) |
| Three drives | A | A to 1 / start to 1 | start-A | **doc** (61.8 % or 78.6 % of the drive) |
| | drive 2 | A to 2 / 1 to A | 1-2 | **doc** (127.2 % or 161.8 %) |
| | C | 2 to C / A to 2 | A-C | **doc** |
| | drive 3 | C to 3 / 2 to C | 2-3 | **doc** |

That the tools *print* exactly these, and on these lines, is **inferred**: TradingView's pages state
the rules the ratios should follow and say the tool "displays the calculated Fibonacci ratios
between the legs", not which. Head and shoulders and Triangle print none.

- They are worked from the stored anchors' real prices, so they are identical on a Regular, Percent,
  Indexed to 100 or Logarithmic axis, and for a bullish or bearish (mirrored) pattern.
- A reference leg with no length has no ratio: the ratio is left out, never shown as NaN or
  infinity. A ratio above 1,000,000 is a degenerate drawing and is left out too. Negative and very
  large prices are only prices.
- The **measured** ratios are all that is shown. The textbook ranges the help quotes are not drawn
  as targets, a pattern is never called valid or invalid, and no pivot is moved to fit a ratio.
- Printed to `precision` places (0 to 6, three by default) with trailing zeros dropped: `0.618`,
  `1.272`, `0.5`, `1`.

### Elliott labels and degrees

The labels of a wave are written by its **degree**, which is part of the drawing's style and so is
saved, synced, kept in a template and can be a tool's default. The nine standard degrees and their
notation are the table in TradingView's own wiki (**wiki**). Letters of the combination waves follow
the case of the degree's correction letters (**inferred**).

| Degree | Impulse | Correction |
|---|---|---|
| Grand Supercycle | ((I)) ((II)) ((III)) ((IV)) ((V)) | ((a)) ((b)) ((c)) |
| Supercycle | (I) (II) (III) (IV) (V) | (a) (b) (c) |
| Cycle | I II III IV V | a b c |
| Primary | ((1)) ((2)) ((3)) ((4)) ((5)) | ((A)) ((B)) ((C)) |
| **Intermediate** (default) | (1) (2) (3) (4) (5) | (A) (B) (C) |
| Minor | 1 2 3 4 5 | A B C |
| Minute | ((i)) ((ii)) ((iii)) ((iv)) ((v)) | ((a)) ((b)) ((c)) |
| Minuette | (i) (ii) (iii) (iv) (v) | (a) (b) (c) |
| Subminuette | i ii iii iv v | a b c |

The wiki calls Cycle "Circle" and Primary "Major"; they are matched by place. The Charting Library
defaults a new wave to degree 7 of its 15, which is Intermediate counting up from the smallest or
down from the largest degree (**lib**). TradingView has six more degrees (Micro, Submicro, Miniscule
below, Submillennium, Millennium, Supermillennium above) whose notation could not be found: they are
**not offered**. Labels use only ASCII letters, digits and parentheses: no circled digits or other
glyphs that a font may lack.

A wave's labels sit beside the pivots they name, above a high and below a low, and stay with their
pivot as it is dragged. The label of the first pivot (where the first wave starts) is empty.

## Cycles

**A cycle is counted in bars, not in clock time.** Its period is the distance between its two
anchors in the chart's bar index (`TimeIndex::index_of`, fractional between bars). TradingView's three
cycle pages position every anchor by a *bar number* (**doc**) and describe the spacing as "set two
vertical lines a specified distance apart"; they do not say whether that is bars or time (**inferred**:
bars). It is also what is useful: a market that is shut at weekends has no bars there, so a cycle of
five bars still lands five bars on, not five days; and a crypto market that never closes counts the
same either way. The anchors stay times, so a cycle follows a timeframe switch to the same moments
and is counted in the new timeframe's bars. Nothing is rewritten.

- **Cyclic lines**: a vertical line at the first anchor and at every `period` bars after it, in the
  direction of the second (anchors placed right to left repeat to the left). Only the repeats in the
  plot are made, never more than 2,000, and none when they are closer than 3 px.
- **Time cycles**: a half circle over each stretch of one period from the first anchor, on a baseline
  at the first anchor's price, over it when the second anchor is higher and under it when lower.
  TradingView's help article for the tool describes only the interval; that the picture is arcs (as
  its icon and the request show) and the baseline and direction are **inferred**. The radius is half
  the arc's pixel width, so it stays round. An arc narrower than 6 px is not drawn.
- **Sine line**: the second anchor defines "the overall amplitude and frequency of the cycle" (**doc**).
  The two anchors are taken as consecutive extremes (**inferred**): the curve passes through both,
  half a period (the anchor distance) apart, with the midline halfway between them in height, and
  continues both ways across the plot. Dragging either anchor changes the period and the amplitude
  together, as the wave must still pass through it. A half period under 1.5 px is not drawn.

All three are built from the projected anchors in screen space (x is linear in the bar index), so on
a logarithmic axis the sine is a sine of the *drawn* height and passes through the anchors as they
are drawn.

### Curves

`ui/pattern_geometry.rs` samples a curve **only over the part of the plot it crosses** and with a step
that keeps it within 0.2 px of the true curve: an arc by the angle one chord may turn through (at
most 256 segments, and a circle of radius 40,000 over an 800 px plot takes four), a sine by its
sharpest bend (at least 0.5 px and at most 4 px apart, at most 6,000 points). So a zoom in or out
never shows a polygon and never costs more than a bounded amount.

## Drawing

The projection is `ui/drawing_layer/pattern_shapes.rs`; the preview while placing and the finished
drawing go through the same function (`Geometry::preview` fills the pivots still to come with the
pointer), so one never differs from the other.

- **Pivot names and ratios** are `PivotLabel`s on the projected `Screen`, laid out when painted
  (only the painter can size text). A pattern's names are plates in its line colour with its text
  colour (**lib**: white 12 px); a ratio is a ringed plate on the chart's surface in the theme's text
  colour, so a drive of 2 is never mistaken for the pivot named 2; a wave's labels are text in the
  text colour (the line's, until changed) with a halo. Each gives way along its push to the labels
  already placed.
- **Hit-testing** is the shape's: its lines, its dashed measuring lines, its fills when the fill is
  on, and its handles first. Labels are not interactive.
- A drawing wholly off the plot is culled by its bounds, which include the labels' reach.

## Style, the dialog and the toolbar

The style is the common one: line colour, thickness and pattern, fill, and the font of the labels
(`style.text`). What is specific is `style.pattern` (`PatternOptions`): pivot names on or off, ratios
on or off, their precision, and a wave's degree. It is left out of a stored drawing while it holds its
default.

- **Properties dialog**: Style (line, fill), **Inputs** (labels, ratios, precision; a wave's degree and
  labels), Text (the font only: size, colour, bold, italic: a pattern writes its own labels), Coordinates
  (each pivot named: X, A, B... or Wave 1..., Start), Visibility.
- **Floating toolbar**: line colour, fill colour, label colour, thickness and pattern as for any line;
  a wave's degree and labels; a pattern's names and ratios. A wave's labels follow its line colour
  until they are given one of their own.
- The Coordinates tab edits each pivot's price and UTC time like any other tool.

## Storage and sync

- A pattern is `Geometry::Pattern { kind, points }`, a wave `Geometry::Wave { kind, points }`, a cycle
  `Geometry::Cycle { kind, a, b }`. A pattern or wave with the wrong number of pivots for its kind is
  a damaged row: it is kept as `Drawing::Unreadable`, never drawn, and written back untouched.
- Drawings sync as part of saved layouts, one record each, like every other drawing: the new tools
  needed nothing in the sync engine. An older build keeps one as `Unreadable` and syncs it back
  untouched ([newer versions](icloud-sync.md#newer-versions)).
- Mocked-sync tests run all fourteen across two Macs on `MockCloud`, change a wave's degree and move a
  pivot of a pattern (each uploads one record and no other), and delete one. **Real CloudKit and two
  real Macs were not exercised.**
- MCP lists the new tools (`xabcd_pattern`, `elliott_impulse`, `sine_line`, ...) and can delete them;
  it cannot create or edit them (`docs/mcp-remaining-work.md`).

## Replay, scales, panes, snapshots

Anchors are times and prices and nothing else: a pan never rewrites one, and the drawings are the
same on every price scale mode (the ratios are worked from prices, and the curves from the drawn
anchors). They are drawn through the bars the chart draws: a pattern may reach into the future of a
replay (its pivots are the user's) but nothing is read from a bar replay has not shown. They are
main-pane drawings. A snapshot draws them through the same code as the live chart, without the
handles.

## Not done, approximate or unverified

- **Nothing was compared with TradingView**, which was not available: every visual (label size and
  plate, fill opacity, handle size, the apex and neckline extents) is chosen here or taken from the
  Charting Library's defaults.
- **Inferred**: the pivot counts of Head and shoulders, Three drives and the Elliott tools; which
  ratios each pattern prints and on which lines; the XABCD colour; the Head and shoulders and Three
  drives fills; the Triangle's run to its apex; the time-cycle arcs, their baseline and direction; the
  sine line's reading of its anchors; bars rather than clock time.
- **Degrees**: six of TradingView's fifteen are not offered (no notation found).
- **Not offered**: TradingView's Elliott "Show Wave" and background options, an automatic Elliott scan,
  pattern validity or target projections, ratio labels you can drag, bearish or bullish colouring by
  direction, and creating or editing these over MCP.
- **Seven coordinates for Three drives** (TradingView lists seven, the help text five pivots and a
  start): six are used.
- **A time cycle's baseline is its first anchor's price** and its direction the side of it the
  second anchor is on, so dragging the second anchor across the first turns it over; a sine's
  amplitude is the height between its anchors.

## Tests

`model/pattern.rs`, `elliott.rs` and `cycle.rs` have hand-worked fixtures (a Gartley, a Cypher, an
ABCD and a Three drives with ratios worked on paper, bullish and mirrored, degenerate legs, negative
and huge prices, every degree against the wiki table, repeats at fractional and negative periods and
at the limit). `ui/pattern_geometry.rs` checks the apex, arcs and sine against the algebra, their
sampling error, and extreme sizes. `drawing_layer/pattern_tests.rs` projects every tool onto the
100 px a bar, 10 px a price plot of the line tests and checks each position by hand, the preview
against the finished drawing, the same labels on a logarithmic scale, and the off-plot and too-dense
cases. A snapshot of each tool is compared shape for shape with the live frame. The every-tool loops of the drawing layer (one gesture is one undo step, Escape cancels, select
move and delete, every handle reshapes, extreme anchors, save and reopen) run all fourteen, as do the
model, store and mocked-sync round trips.

## Manual visual test matrix

Compare against TradingView in both themes at 1x and 2x. The hand-run gallery
(`DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_patterns -- --ignored`) writes every tool
as a PNG from the app's own renderer, which is a way to look at them without a window; it is not a
comparison with TradingView.

| Check | Where |
|---|---|
| Label plates, ratio plates and the halo of a wave's labels over dense candles, dark and light | XABCD, Cypher, impulse |
| Ratios on a log scale and a Percent scale (the same numbers) | XABCD |
| Pivot labels staying clear of each other when pivots are close | a small pattern |
| Neckline and fills with a sloped neckline and uneven shoulders; inverse | Head and shoulders |
| The triangle's lines reaching an apex off screen, and an expanding triangle | Triangle |
| Degree change in the dialog and in the toolbar; labels follow the line colour | any wave |
| Cycles across 1H, 1D and 1W, and on a Renko chart (bricks are the bars) | all three |
| Zooming far in on a time cycle arc, far out on cyclic lines, and a very short sine period | cycles |
| The Patterns menu: width, section headings, scrolling in a short window, keyboard | the tile's corner |
| Placing a seven-pivot drawing with Backspace and a right-click | Head and shoulders |

## Sources

TradingView help: [Drawing tools available on TradingView](https://www.tradingview.com/support/solutions/43000703396-drawing-tools-available-on-tradingview/),
[XABCD](https://www.tradingview.com/support/solutions/43000569909-xabcd-pattern-drawing-tool/),
[Cypher](https://www.tradingview.com/support/solutions/43000791680-cypher-harmonic-pattern-drawing-tool/),
[ABCD](https://www.tradingview.com/support/solutions/43000570202-abcd-pattern-drawing-tool/),
[Head and shoulders](https://www.tradingview.com/support/solutions/43000570149-head-and-shoulders-drawing-tool/),
[Triangle pattern](https://www.tradingview.com/support/solutions/43000570208-triangle-pattern-drawing-tool/),
[Three drives](https://www.tradingview.com/support/solutions/43000570172-three-drives-pattern-drawing-tool/),
[Cyclic lines](https://www.tradingview.com/support/solutions/43000518161-cyclic-lines-drawing-tool/),
[Time cycles](https://www.tradingview.com/support/solutions/43000791682-time-cycles-drawing-tool/),
[Sine line](https://www.tradingview.com/support/solutions/43000791685-sine-line-drawing-tool/),
[Elliott wave theory and tools](https://www.tradingview.com/support/solutions/43000653212-elliott-wave-theory-and-tools/).
Charting Library: the per-tool override interfaces (`AbcdLineToolOverrides`,
`CypherpatternLineToolOverrides`, `HeadandshouldersLineToolOverrides`, `TrianglepatternLineToolOverrides`,
`ThreedriversLineToolOverrides`, `Elliott*LineToolOverrides`, `TimecyclesLineToolOverrides`,
`SinelineLineToolOverrides`, `CirclelinesLineToolOverrides`) for the defaults. The degree notation is
the table in TradingView's wiki for its Elliott tools. The help pages were read through a summarising
fetch, not in a browser, so a detail a page states only in an image is missing from this page.
