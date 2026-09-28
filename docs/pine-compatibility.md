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

- Required `//@version=6` and exactly one `indicator()` declaration.
- Declaration arguments: `title`, `shorttitle`, `overlay`, `format`, `precision`,
  `max_bars_back`, and `max_lines_count`/`max_labels_count`/`max_boxes_count` (default
  50; the oldest drawing is deleted past the limit). Other valid arguments report `PINE9001`.
- Integers, floats, booleans, strings, colors, typed `na`, declarations, `var`, `varip`,
  reassignment and compound numeric assignment.
- Arithmetic (int-preserving for `+ - * %`), string concatenation, comparison, lazy
  `and`/`or`, unary operators, ternary expressions, member names, history references,
  named arguments, indentation, and wrapped calls/expressions.
- `if` / `else if` / `else` chains; `for i = a to b [by s]` (direction follows the
  bounds), `for x in array`, `for [i, x] in array`, `break`, and `continue`.
- User-defined functions with typed/untyped parameters, defaults, named arguments,
  single-line or block bodies (last statement is the result). Locals are scoped to the
  call; builtins inside a function keep separate history per call site; `var` locals
  persist per call site. Functions cannot reassign globals (`PINE3022`).
- Type annotations: `int`, `float`, `bool`, `string`, `color`, `line`, `label`, `box`,
  `table`, `T[]`, and `array<T>`.
- v6 boolean rules: booleans are non-nullable and numeric values are not conditions.
- Market values: `open`, `high`, `low`, `close`, `volume`, `hl2`, `hlc3`, `ohlc4`, `time`,
  `time_close`, `bar_index`, and `syminfo.mintick` (inferred from price decimals).
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
- TA entry points: SMA, EMA, RMA, RSI, MACD, ATR, change, highest, lowest, cross,
  crossover, crossunder.
- Visual entry points: plot (per-bar color, returns a handle), fill (between two plots,
  per-bar color), hline, plotshape (incl. `shape.circle`, `location.absolute`, `size`),
  plotchar, bgcolor, and barcolor.
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

## Known incompatibilities

The current grammar does not yet implement tuple destructuring, `switch`, `while`, method
call syntax (`arr.push(x)`), maps, matrices, user-defined types, enums, `polyline`,
`linefill`, or `xloc.bar_time` drawings. Label `yloc` is treated as `yloc.price`.
Qualifier metadata types exist, but full compile-time overload/qualifier inference is not
yet complete. Stateful TA warm-up matches the documented seed approach for common data,
but missing-value and conditional-call behavior needs a larger differential corpus. Non-overlay
scripts draw in their own pane (bottom 30% of the card) with a value axis fitted to their
visible outputs; `barcolor` still recolors the price candles. The pane has no crosshair,
and overlay values do not yet join price autoscaling. Plot style/location/size
coverage is partial. Runtime byte accounting, recursion detection, and a compact bytecode
lowering pass are planned; the current executable representation is the typed AST.

`request.security`, alerts, strategies, libraries, maps/matrices, and `switch` are
intentionally outside this release and produce unsupported or
unknown-function diagnostics. REST reconciliation reevaluates the visible canonical
series. Binance carries explicit close flags and accepts new-bar transitions; Alpaca bars
are still reconciled through the existing timeframe aggregator.

## Conformance and performance

`PineEngineTests` executes the six integration scripts from `PINE.md` over deterministic
OHLCV fixtures and checks history, persistent state, v6 diagnostics, realtime rollback,
and `varip`. The entire pre-existing test target remains the regression gate. A formal
TradingView differential corpus and an Instruments peak-memory run are still required
before publishing compatibility or 100,000-bar benchmark numbers; no unmeasured numbers
are claimed here.
