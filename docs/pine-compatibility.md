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

### Execution model

Execution is event-driven. Market data decides when a script runs; the runtime decides how.

```
market update → PineCandleAggregator → PineExecutionScheduler → PineRuntimeSession.execute
   (WS tick, REST      bar identity,        does this script       rollback, run top to bottom,
    refresh, live bar)  dedupe, lifecycle    run for this event?    commit when the bar is confirmed
```

Each chart owns one `PineExecutionHost` (an actor around a `PineExecutionController`, which owns
the aggregator and the session). There is no timer and no polling inside Pine: events come from
the existing WebSocket callback, the existing 5 s refresh, and rebuild triggers.

- **Load / rebuild.** A new host recalculates from scratch: closed bars run as `.historical` and
  commit one by one; a still-forming final bar runs as the first realtime tick (`isnew`). A rebuild
  happens on script apply or edit, input or theme change, symbol or timeframe change, replay
  entry or exit, and whenever the feed can no longer be reconciled with committed history. No state
  crosses a rebuild.
- **Bar identity** is the bar's open time within a dataset (`PineDatasetKey`: `<source>:<ticker>`
  plus timeframe). The aggregator drops duplicates, late ticks, and repeats of an already closed
  bar. `PineRuntimeSession.execute` independently refuses (`PineExecutionRejection`) a bar older
  than the last one executed or one already committed.
- **Realtime.** Each tick runs on a copy of the last committed state (`working = committed`), so
  ordinary variables, `var`, call-site histories, arrays, plots, drawings, and the broker roll back.
  `varip` values live in `session.intrabar` and are restored into each recalculation of the same
  bar. Only the closing execution commits, so a bar contributes exactly one series entry no matter
  how many ticks it saw.
- **Close** is confirmed by the provider's flag (Binance `x`) or, for REST-only sources, by the
  next bar arriving. It is never inferred from a clock.
- **Scheduling** (`PineExecutionScheduler`): indicators and libraries execute on every event.
  Strategies execute on history and on each realtime bar's closing update; with
  `calc_on_every_tick=true` they also execute on every tick, with the same rollback. Ticks a
  strategy skips still update the aggregator.
- **Alerts** are raised during execution and collected per execution. Historical executions
  record alerts (they appear in the report list) but never emit them for delivery; only
  executions on a realtime bar do. `alert.freq_once_per_bar` uses a per-bar ledger that survives
  rollback, `freq_once_per_bar_close` requires the closing execution, `freq_all` fires on every
  execution. Events carry `frequency`, `isRealtime`, and `isConfirmed`.
- **Multiple scripts** have separate hosts, sessions, and aggregators; nothing mutable is shared.
  The chart UI applies one script per chart today.

`barstate.*` follows the [Pine execution model](https://www.tradingview.com/pine-script-docs/language/execution-model/):
on a historical bar `isnew` and `isconfirmed` are both true. On a realtime bar the first execution
has `isrealtime`, `isnew`, and not `isconfirmed`; later ticks have neither `isnew` nor
`isconfirmed`; the closing execution has `isconfirmed` and not `isnew`. In this engine a default
strategy executes only on the closing update, so there `isnew` and `isconfirmed` are both true;
that combination was derived from the rules above, not checked against TradingView.

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
  lists. Only events raised on realtime bars can notify, and only through a **script alert**
  subscription (see below). `alert.freq_once_per_bar_close` fires only on confirmed bars. The
  compiler checks the calls: `alert` takes a message and an optional `alert.freq_*` constant
  (`PINE3025`, `PINE3027`, `PINE3028`), `alertcondition` a condition, title and message
  (`PINE3026`, `PINE3027`), and a known non-string message is rejected (`PINE3036`). A frequency
  the script computes that resolves to nothing fails the run (`PINE4009`) instead of silently
  defaulting.

#### Script alerts (notifications)

**Create Alert…** in a chart's Scripts tab subscribes to the applied script's alerts. A
subscription is pinned to that chart, symbol, timeframe, and the SHA-256 of the applied source.
Delivery is a macOS notification and an in-app banner, governed by the same settings as price
alerts. The path is `PineExecutionUpdate.alerts` → `ChartViewModel.pineAlertHandler` →
`PineAlertCoordinator` → `PineAlertRouter` → `PineAlertFrequencyGuard` → `PineAlertStore` (history
row) → `PineAlertDispatcher` (channels).

- **History never notifies.** The runtime flags historical executions `isRealtime = false` and
  drops alerts raised while loading or rebuilding; the router drops any non-realtime event, and a
  bar that ended before the subscription was created is ignored. Replay never goes live.
- **Dedupe.** Each admitted `once_per_bar` / `once_per_bar_close` alert is stored with the key
  `subscription|call site|bar open (ms)|frequency`, unique in `pine_alert_event`, and the guard is
  seeded from those keys at launch, so a relaunch cannot deliver the same bar twice. `freq_all`
  has no key: it may fire on every execution.
- **Edit.** Applying different source (or clearing the script) sets the subscription to *Script
  changed*. It stays silent until you re-arm it, which re-pins the chart's current script, symbol
  and timeframe and clears its dedupe keys.
- **Delete.** Removing the saved script sets *Script deleted*; removing the chart deletes its
  subscriptions.
- **Symbol or timeframe change.** The subscription stays enabled but fires only while its chart
  shows the pair it was created for; the Alerts center shows "Chart shows other symbol".
- **Channels.** `PineAlertChannel` is the seam: a webhook would be one more conformance. Channels
  run independently and a failing one never reaches the script or blocks the others.
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
(ticks), `pyramiding`, `process_orders_on_close`, `calc_on_every_tick`, and `currency`.
`calc_on_order_fills`, `close_entries_rule`, `margin_*`,
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

## Identifier rules

`PineIdentifierRules` checks every name a script declares: variables, tuple names, function names
and parameters, and `for` counters. Codes follow TradingView's numbering.

| Code | Rule |
|---|---|
| `CE10090` | A declared name contains `.`: `math.max = 44`. |
| `CE10190` | A declared name shadows a builtin variable or function: `close = 5`, `f(high) =>`, `plot = 1`. |

A bare namespace is not a builtin variable, so `math = 44`, `ta = 1` and `color = 44` are allowed,
as are the object-type names `line`, `label`, `box` and `table`. `_` is always allowed. TradingView
only errors on `CE10190` when the script has already used the builtin and otherwise warns
(`CW10011`); DegenView always errors. Dotted `:=` (`obj.field := 1`) is not a declaration and stays a
syntax error until user-defined types exist.

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
enumeration constants. An integer literal too large for an `int` is `PINE2014`. A REST refresh is
reconciled against committed bars (`PineExecutionController.sync`): new bars become close and open
events, an identical snapshot changes nothing, and a snapshot that disagrees with committed history
(corrected bar, missing bar, longer window) rebuilds the script. Binance carries explicit close
flags; other sources infer a close from the next bar.

### Known differences from TradingView (realtime execution)

Verified against the official execution-model and strategy pages only where stated; nothing else is
claimed as parity.

- **Order fills on realtime bars.** Orders fill at the next bar's open on history and realtime alike,
  including with `calc_on_every_tick=true`. TradingView fills on the next realtime tick, and order
  data produced on realtime ticks is not rolled back there; here the broker rolls back with the rest
  of the state. `calc_on_order_fills` is accepted and ignored.
- **`varip` and every-tick strategies are not reproducible from history.** After a reload only
  OHLCV bars exist, so each historical bar is one execution. A `varip` counter that reached 4 while
  the bar was live starts at 1 on recalculation, and an every-tick strategy sees one execution per
  bar. Recalculation is deterministic for a given source, inputs, and bars; realtime-only state never
  leaks into it.
- **Close detection** uses the stream flag or the next bar. A WebSocket reconnect can miss a bar's
  closing message; the next REST refresh reconciles (a differing final value rebuilds), but
  an alert for that bar's close is not sent retroactively.
- **A forming bar at load** runs as a realtime tick only when its open time and the bar length say it
  is still open. Indicators execute it at once; a default strategy waits for its close.
- **Sources without a stream** (CoinGecko, DEX pairs) update once per 5 s refresh, so an indicator
  sees at most one realtime execution per refresh, and `freq_all` alerts fire per execution, not per
  trade. Alpaca folds minute bars into the chart bar and feeds that as a stream tick.
- **Rollback cost.** State is two value copies. The first append to each history or plot array in a
  recalculation copies it, so a tick costs O(bars) and a full load O(bars²) (see above). Fine at
  chart sizes; a journaled or truncating rollback is the next step if it matters.
- **One script per chart** in the UI; the engine itself supports any number.
- **Script alerts are client-side.** They need the app running and the chart's tab visible (hidden
  tabs suspend the feed), and a bar close missed while disconnected is not alerted retroactively.
  There is no server worker, webhook, email, push, or account.

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
