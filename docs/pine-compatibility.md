# Pine v6 indicator-engine compatibility

This is an independent, local indicator engine. It does not execute code on TradingView
or claim complete Pine Script compatibility. Semantics were based on TradingView's v6
Execution Model, Type System, Operators, Variable Declarations, Bar States, Inputs, Plots,
and Reference Manual as reviewed on 2026-08-24.

## Architecture

`PineCompiler.compile(source:)` runs a ranged lexer, Pratt expression parser, AST builder,
declaration/input analysis, and semantic validation. A valid `PineCompiledProgram` is fed
to `PineRuntimeSession`; source text is never evaluated. The VM executes every statement
in source order once per bar and stores variable and call-site histories independently.
Builtins are dispatched by namespace and stable parsed call-site ID. Visual calls append
to renderer-neutral output series consumed by the chart Canvas.

Historical executions commit after each bar. An open realtime execution starts from the
last committed state; subsequent ticks roll ordinary variables, call histories, and
visuals back before executing again. `varip` state survives those rollbacks. Only a closing
event commits. The runtime exposes `barstate.isfirst`, `islast`, `ishistory`, `isrealtime`,
`isnew`, `isconfirmed`, and `islastconfirmedhistory`. Replay passes only `replayKlines`, so
canonical future bars are unavailable to scripts.

## Supported

- Required `//@version=6` and exactly one `indicator()` or `strategy()` declaration.
- Declaration arguments: `title`, `shorttitle`, `overlay`, `format`, `precision`,
  `max_bars_back`, and `max_lines_count`/`max_labels_count`/`max_boxes_count` (default
  50; the oldest drawing is deleted past the limit). Arguments may be constants, named
  constants (`strategy.percent_of_equity`), or earlier `const` variables. Other valid
  arguments report `PINE9001`.
- Integers, floats, booleans, strings, colors, typed `na`, declarations, `var`, `varip`,
  reassignment and compound numeric assignment.
- Arithmetic (int-preserving for `+ - * %`), string concatenation, comparison, lazy
  `and`/`or`, unary operators, ternary expressions, member names, history references,
  named arguments, indentation, and wrapped calls/expressions.
- `if` / `else if` / `else` chains; `for i = a to b [by s]` (direction follows the
  bounds), `for x in array`, `for [i, x] in array`, `while`, `break`, and `continue`.
  `switch` (with or without a subject, optional `=>` default arm), and `if`/`switch` used
  as expressions (`x = switch …`). Tuple destructuring `[a, b] = f()`.
- `const`, `simple`, and `series` qualifiers before declarations.
- User-defined functions with typed/untyped parameters, defaults, named arguments,
  single-line or block bodies (last statement is the result). Locals are scoped to the
  call; builtins inside a function keep separate history per call site; `var` locals
  persist per call site. Functions cannot reassign globals (`PINE3022`).
- Type annotations: `int`, `float`, `bool`, `string`, `color`, `line`, `label`, `box`,
  `table`, `T[]`, and `array<T>`.
- v6 boolean rules: booleans are non-nullable and numeric values are not conditions.
- Market values: `open`, `high`, `low`, `close`, `volume`, `hl2`, `hlc3`, `ohlc4`, `time`,
  `time_close` (the bar's open time plus the bar length, learned from consecutive bars),
  `bar_index`, and `syminfo.mintick` (inferred from price decimals).
  `chart.fg_color`/`chart.bg_color` follow the app's light or dark appearance.
- `na()`, `nz()`, `int()`/`float()`/`bool()` casts, `math.max/min/abs/round/floor/ceil/sign/
  sqrt/pow/log/exp`, color constants, `color.new`, `color.rgb`, and `str.tostring` with
  `#`/`0` patterns, `format.mintick`, and `format.percent`.
- Arrays: `array.new_<type>`, `array.from`, `push`, `unshift`, `get`, `set`, `insert`,
  `remove`, `size`, `shift`, `pop`, `first`, `last`, `clear`, `includes`, `indexof`, `sum`,
  `avg`, `max`, `min`. Out-of-range access reports `PINE4010`. Arrays and drawings are
  part of bar state and roll back with realtime ticks.
- Inputs: int, float, bool, string, and color defaults (literals, `color.*` constants,
  constant `color.new`/`color.rgb`) plus title, tooltip, group, inline, confirm, min/max,
  step, and `options` (rendered as a picker). Input values reevaluate without recompilation.
- TA entry points: SMA, EMA, RMA, WMA, RSI, MACD, ATR, TR, stdev, bb, mom, roc, change,
  highest, lowest, rising, falling, cum, barssince, cross, crossover, crossunder, and
  `pivothigh`/`pivotlow` (2- and 3-argument forms). A pivot appears on the bar that
  confirms it, `rightbars` after the pivot bar. The centre must be strictly beyond every left
  bar and at least as extreme as every right bar, so a flat top yields one pivot at its first
  bar; this tie rule is an assumption not yet checked against TradingView.
- Also: `math.avg/log10/sin/cos/tan/asin/acos/atan/todegrees/toradians/round_to_mintick`,
  `math.pi/e/phi`; `str.length/contains/upper/lower/trim/startswith/endswith/replace_all/
  substring/tonumber/format`; `array.sort/reverse/copy/concat/slice/join`; `timestamp()`
  (date strings, numeric parts, optional zone; UTC when none is named), `year`, `month`,
  `dayofmonth`, `hour`, `minute`, `second`, `dayofweek` (variables and functions),
  `syminfo.ticker/tickerid/currency/type`, and `timeframe.*` inferred from bar spacing.
- `input.time` (rendered as a date picker) and `group` headings in the Scripts tab.
- Visual entry points: plot (per-bar color, returns a handle; styles line, linebr, stepline,
  histogram, columns, area, circles, cross), fill (between two plots, per-bar color, or the
  gradient overload `fill(p1, p2, top_value, bottom_value, top_color, bottom_color)`), hline,
  plotshape (incl. `shape.circle`, `location.absolute`, `size`), plotchar, plotcandle, bgcolor,
  and barcolor. `display` is honoured on plot, plotshape, plotchar, and plotcandle; a
  `display.none` plot is not drawn but a `fill()` may still reference it.
- `alert()` and `alertcondition()` record an event (bar, time, message) that the Scripts tab
  lists. They never notify. `alert.freq_once_per_bar_close` fires only on confirmed bars.
- Drawing objects with `xloc.bar_index` coordinates: `line.new/set_*/get_*/delete`
  (style, extend), `label.new/set_*/get_*/delete` (bubble styles up/down/left/right/none),
  `box.new/set_*/get_*/delete`, and `table.new/cell/delete` pinned to any `position.*`.
- Canvas order: backgrounds, volume, candles, fills, boxes, plots, lines, hlines,
  markers, labels; tables draw above the series outside its clip.
- Per-card persisted draft, last-valid source, and typed inputs. Invalid drafts and their
  line/column diagnostics survive while last-valid output stays active.
- Each statement must end its line: leftover tokens after a complete statement
  (`aaa "x" 1`, `plot(close) 5`) are a `PINE2013` syntax error, reported once per line.
- Limits: 100k source characters, 50k tokens/nodes, 100k IR/executed instructions,
  64 call depth/visuals, 1m history bars, 256 MB declared runtime budget, cooperative
  cancellation, and a 10-second evaluation deadline. Enforced limits use `PINE8xxx`.

## Type checking

`PineTypeChecker` runs after parsing (skipped when the source has lexical or syntax errors) and
reports compile errors for:

| Code | Rule |
|---|---|
| `PINE3030` | Initializer does not fit the declared type: `const string g = 222`, `int n = 1.5`, `bool b = 1`. `int` fits `float`; `na` fits anything (`bool x = na` is `PINE3021`). |
| `PINE3031` | Initializer's qualifier is higher than the declared `const`/`simple`: `const int n = input.int(5)`. |
| `PINE3032` | `:=` or a compound assignment changes the variable's type: `x = 0` then `x := 1.5`. |
| `PINE3033` | `if`/`while` condition, ternary test, subjectless `switch` arm, or `and`/`or`/`not` operand is not bool. |
| `PINE3034` | Operator applied to unsuitable operands: `"a" - 1`, `"a" + 1`, `close < "x"`. Comparisons with `na` are always allowed. |
| `PINE3035` | `input.*` default does not fit its function: `input.int("x")`, `input.bool(1)`. |

The checker only reports when every type involved is certain. It knows literals, operators,
market series, `barstate.*`/`syminfo.*`/`strategy.*` values, `input.*`, and the return types of
common `ta.*`, `math.*`, `str.*`, `color.*`, and casts. Everything else (user-function results,
other builtins, arrays, tuples, undeclared names) is treated as unknown and never flagged.
An unannotated variable keeps its initializer's qualifier, so `grp = "G"` can feed `group = grp`;
`var`/`varip` variables and anything reassigned with `:=` are series. Builtin argument
signatures and undeclared identifiers are not checked; the runtime still rejects bad values it
meets (`PINE4001`–`PINE4006`).

## Strategies

`strategy()` scripts run through a broker emulator (`PineBrokerEmulator`). Settings honoured:
`initial_capital`, `default_qty_type`/`default_qty_value` (fixed, cash, percent of equity),
`commission_type`/`commission_value` (percent, cash per order, cash per contract), `slippage`
(ticks), `pyramiding`, `process_orders_on_close`, and `currency`. `margin_*`,
`use_bar_magnifier` and other arguments report `PINE9001`.

- Orders queued on bar N fill on bar N+1 (or at bar N's close with `process_orders_on_close`).
  Market orders fill at the open; stop and limit orders fill where the bar's price path
  reaches them, using TradingView's assumption that the path goes from the open to the nearer
  extreme first. A stop the open gaps through fills at the open. Slippage is adverse on
  market and stop fills, never on limit fills.
- `strategy.entry` in the opposite direction closes the position and opens the new side.
  Same-direction entries are limited by `pyramiding` (0 and 1 both allow one entry).
  `strategy.order` nets against the position and does not apply pyramiding.
- Supported calls: `entry`, `order`, `exit` (`from_entry`, `stop`, `limit`, `loss`/`profit`
  ticks, `qty`, `qty_percent`), `close`, `close_all`, `cancel`, `cancel_all`; `strategy.risk.*`
  is accepted and ignored. Trailing stops report `PINE9005`.
- Series: `strategy.position_size`, `position_avg_price`, `equity`, `netprofit`, `openprofit`,
  `initial_capital`, `closedtrades`, `opentrades`, `wintrades`, `losstrades`, `grossprofit`,
  `grossloss`. The first five keep history, so `strategy.position_size[1]` works.
- Not modelled: margin and leverage, contract-size rounding, the bar magnifier, and
  stop-limit fills to the tick. Results will not match TradingView exactly. The runtime
  copies its history arrays each bar, so evaluation time grows quadratically with bar count:
  about 2 s for 1,500 bars of a script this size, and it reaches the 10-second deadline
  somewhere before 5,000 bars.

## Known incompatibilities

The current grammar does not yet implement method call syntax (`arr.push(x)`), maps,
matrices, user-defined types, enums, `polyline`, `linefill`, or `xloc.bar_time` drawings. Label `yloc` is treated as `yloc.price`.
Qualifier metadata types exist, but full compile-time overload/qualifier inference is not
yet complete. Stateful TA warm-up matches the documented seed approach for common data,
but missing-value and conditional-call behavior needs a larger differential corpus. Non-overlay
scripts draw in their own pane (bottom 30% of the card) with a value axis fitted to their
visible outputs; `barcolor` still recolors the price candles. The pane has no crosshair,
and overlay values do not yet join price autoscaling. Plot style/location/size
coverage is partial. Runtime byte accounting, recursion detection, and a compact bytecode
lowering pass are planned; the current executable representation is the typed AST.

`request.security`, libraries, and maps/matrices are
intentionally outside this release and produce unsupported or
unknown-function diagnostics; a `request.*` call is reported (`PINE9003`) wherever it
appears, including inside an assignment or argument. Reading a plain identifier that is
not a variable, series, or builtin raises `PINE4008` at runtime instead of silently
evaluating to a string; dotted names such as `size.small` or `shape.circle` remain
enumeration constants. An integer literal too large for an `int` is `PINE2014`. REST reconciliation reevaluates the visible canonical
series. Binance carries explicit close flags and accepts new-bar transitions; Alpaca bars
are still reconciled through the existing timeframe aggregator.

## Conformance and performance

`PineRegressionTests` pins one case per defect fixed while the engine was split into the
`DegenView/Pine` folder (overflow and range traps, UTF-16 diagnostic offsets, `time_close`,
month-name matching, named array arguments). `PineEngineTests` executes the six integration scripts from `PINE.md` over deterministic
OHLCV fixtures and checks history, persistent state, v6 diagnostics, realtime rollback,
and `varip`. `PineStrategyTests` runs a full volume-breakout strategy verbatim plus
focused language and broker cases (fills, gaps, commissions, pyramiding, rollback). The
entire pre-existing test target remains the regression gate. A formal
TradingView differential corpus and an Instruments peak-memory run are still required
before publishing compatibility or 100,000-bar benchmark numbers; no unmeasured numbers
are claimed here.
