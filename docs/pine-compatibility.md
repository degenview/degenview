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
- Declaration arguments: `title`, `shorttitle`, `overlay`, `format`, `precision`, and
  `max_bars_back`. Other valid arguments report `PINE9001`.
- Integers, floats, booleans, strings, colors, typed `na`, declarations, `var`, `varip`,
  reassignment and compound numeric assignment.
- Arithmetic, comparison, lazy `and`/`or`, unary operators, ternary expressions, member
  names, history references, named arguments, indentation, and wrapped calls/expressions.
- v6 boolean rules: booleans are non-nullable and numeric values are not conditions.
- Market values: `open`, `high`, `low`, `close`, `volume`, `time`, `time_close`, and
  `bar_index`.
- `na()`, `nz()`, common `math.*`, color constants, `color.new`, and `color.rgb`.
- Inputs: int, float, bool, and string defaults plus title, tooltip, group, inline,
  confirm, min/max, and step metadata. Input values reevaluate without recompilation.
- TA entry points: SMA, EMA, RMA, RSI, MACD, ATR, highest, lowest, crossover, crossunder.
- Visual entry points: plot, hline, plotshape, plotchar, bgcolor, and barcolor. Overlay
  plots, backgrounds, candle colors, horizontal lines, and markers render in deterministic
  Canvas order.
- Per-card persisted draft, last-valid source, and typed inputs. Invalid drafts and their
  line/column diagnostics survive while last-valid output stays active.
- Limits: 100k source characters, 50k tokens/nodes, 100k IR/executed instructions,
  64 call depth/visuals, 1m history bars, 256 MB declared runtime budget, cooperative
  cancellation, and a 10-second evaluation deadline. Enforced limits use `PINE8xxx`.

## Known incompatibilities

The current grammar does not yet implement user-defined function bodies, tuple
destructuring, `switch`, loops, methods, collections, objects, enums, or drawing objects.
Qualifier metadata types exist, but full compile-time overload/qualifier inference is not
yet complete. Stateful TA warm-up matches the documented seed approach for common data,
but missing-value and conditional-call behavior needs a larger differential corpus. The
non-overlay 65/35 synchronized pane and Pine values in price autoscaling are not yet
implemented; non-overlay output currently uses the price canvas. Plot style/location/size
coverage is partial. Runtime byte accounting, recursion detection, and a compact bytecode
lowering pass are planned; the current executable representation is the typed AST.

`request.security`, alerts, strategies, libraries, arrays/maps/matrices, loops, `switch`,
and drawing objects are intentionally outside this release and produce unsupported or
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
