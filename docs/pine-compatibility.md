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

- Required `//@version=6` and exactly one `indicator()`, `strategy()` or `library()` declaration.
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
- Drawing objects with `xloc.bar_index` coordinates (or `xloc.bar_time`, see below): `line.new/set_*/get_*/delete`
  (style, extend), `label.new/set_*/get_*/delete` (bubble styles up/down/left/right/none),
  `box.new/set_*/get_*/delete`, and `table.new/cell/delete` pinned to any `position.*`.
- Canvas order: backgrounds, volume, candles, fills, boxes, plots, lines, hlines,
  markers, labels; tables draw above the series outside its clip.
- Per-card persisted draft, last-valid source, and typed inputs. Invalid drafts and their
  line/column diagnostics survive while last-valid output stays active.
- Each statement must end its line: leftover tokens after a complete statement
  (`aaa "x" 1`, `plot(close) 5`) are a `PINE2013` syntax error, reported once per line.
- Limits: 500k source characters (a runaway-input guard; published scripts reach 300k), 50k
  tokens and AST nodes, 20M executed instructions per bar (Pine bounds a bar by time, not by steps),
  64 call depth/visuals, 1m history bars, 256 MB declared runtime budget, cooperative
  cancellation, and a 10-second evaluation deadline checked between bars. Enforced limits use `PINE8xxx`.

### Language additions

- Syntax found in community scripts: single-quoted strings (`'…'`), continuation lines indented by
  a non-multiple of four columns (multi-line ternaries and operator chains), several statements on one
  line separated by commas (`a := 1, b := 2`), generic constructors (`array.new<float>(n)`,
  `map.new<K, V>()`, `matrix.new<T>(r, c)`), the `export` modifier (accepted in a `library()`,
  `PINE3037` elsewhere), and a type keyword as a variable name (`color = x > 1 ? color.green : color.red`).
- `input.*` defaults are read from `defval =` (or the leading positional argument), may be named
  constants (`input.string(size.small, …)`), and `options` may be the third positional argument.
- A subscript on a call or an expression (`ta.highest(high, 5)[1]`, `(a + b)[2]`) reads that
  expression's own history, recorded on every bar it is reached. `hl2`, `hlc3`, `ohlc4`, `hlcc4`, `time`,
  `time_close` and `bar_index` keep a history too, so `time[1]` works.
- Casts and helpers: `color(x)` and `string(x)` (a value of that type passes through, anything else is
  `na`), `max_bars_back` (a no-op: all history is kept), `str.match` (first match or `""`), `str.split`,
  `str.repeat`, `array.sort_indices`, `color.from_gradient`, `color.r/g/b/t`.
- In a condition (`if`, `while`, `?:`, `not`, `and`/`or`) `na` is false, as in v6 where a bool is never `na`:
  a bool series read before its first bar, or a branch not reached, is false. Numbers and strings are still
  not conditions (`PINE4001`–`PINE4005`).
- The legacy generic `input(defval, …)` is accepted; its type is the default's (a series name makes it a
  source input). `runtime.error(message)` stops the script with `PINE4030` and the script's message.
  `ticker.new/standard/modify/inherit` build symbol ids; since only the chart's own symbol can be served,
  `standard`, `modify` and `inherit` return what they are given.
- Declaration arguments `behind_chart` (either value), `explicit_plot_zorder` and `dynamic_requests` are
  accepted and ignored; `max_polylines_count` is honoured. Arguments that change behaviour (`scale`,
  margin and so on) stay `PINE9001`. Constants fold through `const` variables and named constants
  (`const color BASE = …`, `color.new(BASE, 88)`, `const string TINY = size.tiny`), so inputs may default to them.

### Types, methods and collections

- **User-defined types.** `type Name` with typed fields and constant-or-expression defaults (also
  `export type`), `Name.new(positional, named = …)`, field reads (`zone.top`, `zone.origin.y`,
  `array.get(zones, i).top`), field assignment (`:=`, `+=`, also through a call result), `obj.copy()`
  (shallow), and `array<Name>`. Objects are references: two variables can name one instance, and
  instances live in the runtime state, so `var` persistence and realtime rollback behave as for
  arrays. A type must be declared before its first use, as in Pine.
- **Methods.** `method name(Type this, …) =>`, called as `value.name(args)` or `name(value, args)`. The
  receiver may be a variable, a field path or any expression (`zones.get(k).kill()`,
  `holders.get(1).xs.push(8.0)`); it is evaluated once. A method applies only to receivers its first
  parameter accepts (a script-defined type or enum by name; a primitive, array, map, matrix or handle
  by kind; an untyped parameter accepts anything) and then wins over a builtin of the same name, so
  `size(Box this)` does not affect `size()` on an array. The same method name may be defined for
  several receiver types. Builtin methods work the same way: `values.get(i)` is `array.get(values, i)`,
  `ln.set_y2(p)` is `line.set_y2(ln, p)`.
- **Enums.** `enum Name` with `member [= "title"]` lines (also `export enum`). A value is the string
  `"Name.member"`, so `==`, `switch` and `?:` work as for strings; `str.tostring(value)` returns the
  title (the name when there is none). A declared enum name can annotate a variable or parameter.
  `input.enum(Name.member, title, options = [...])` is a string input whose options are the enum's
  members (or the listed ones), shown by title in the inputs panel and stored as `"Name.member"`.
- **Maps.** `map<K, V>` (also nested, `map<string, array<float>>`) with `put`, `get`, `contains`,
  `remove`, `size`, `clear`, `keys`, `values`, `copy`, `put_all`, as functions or methods, including
  through fields (`book.levels.put(k, v)`). Insertion order is kept; `for [k, v] in map` iterates a
  snapshot of the pairs.
- **Matrices.** `matrix<T>` with `get`, `set`, `rows`, `columns`, `row`, `col`, `add_row`, `add_col`,
  `remove_row`, `remove_col`, `fill`, `copy`, `transpose`, `elements_count`, `avg`, `min`, `max`, as
  functions or methods (`m.row(1).avg()`).
- Runtime errors: reading a field of, or calling a method on, `na` is `PINE4018`; an unknown field
  `PINE4019`; assigning through a non-object `PINE4020`; a member the enum lacks `PINE4025`; a `na`
  map or matrix `PINE4026`/`PINE4028`; a `na` map key `PINE4027`; no method definition fits the receiver
  `PINE4029`.

### Libraries and `import`

- **`import user/Library/version as alias`** (alias optional, defaulting to the library name) links another
  script's `library()` into the importing script. The compiler resolves the path through a
  `PineLibraryResolver`; the app's resolver is `PineLibraryRegistry`, which offers the library scripts of the
  Script Manager. Without a resolver (the alert agent, a bare `PineCompiler.compile`) every import is
  `PINE3040`.
- **What a library exports** is reached as `alias.name`: functions (`alias.f(x)`), constants
  (`alias.RATE`), types (`alias.Pt.new(…)`, and `alias.Pt` as an annotation, a field type or a generic argument),
  enum members (`alias.Mode.fast`) and methods (`point.scaled(2)`, picked by the receiver's type like any
  method). Anything the library does not mark `export` is private: `alias.helper(x)` is `PINE3044` at compile
  time (`PINE4031` if it only shows at run time).
- **A library runs in its own scope.** Its functions call their private helpers, its default parameter values
  and type field defaults evaluate in the library, and its code can see only its own locals and constants,
  never the importing script's variables. A library and the script (or two libraries) may define the same
  names; an instance of a library type is a different type from the script's own of the same name.
- **Libraries may import libraries**, up to 8 levels deep. A cycle is `PINE3041`, a deeper chain `PINE3047`.
- **Diagnostics at the `import` line:** `PINE3040` library not found, `PINE3042` malformed import (the path
  must be `user/Library/version` with a numeric version), `PINE3043` the script is not a library or the library
  has errors (the first one is quoted), `PINE3045` an alias that is already taken or names a Pine namespace
  (`math`, `ta`, …). `PINE4032`: a library constant defined in terms of itself.

### Time, timeframes and other series

- `last_bar_index`, `last_bar_time`, `timenow`, `time_tradingday`, `timeframe.isticks`,
  `timeframe.in_seconds([tf])`, `timeframe.change(tf)`, `time(timeframe)` and `time_close(timeframe)`,
  `syminfo.root`, `syminfo.prefix`, `syminfo.timezone`, `syminfo.pointvalue`, `chart.is_standard` and
  the other `chart.is_*` flags.
- **`request.security` for the chart's own symbol**, on a timeframe at least as long as the chart's
  (`"60"`, `"D"`, `"3M"`, `timeframe.period`, `""` for the chart's own). The chart's bars are folded into
  higher-timeframe candles and the expression runs on that series in a state of its own: its
  histories, `var`s and `ta.*` calls are not shared with the chart script, and a realtime tick rolls it
  back like any other state. `gaps` and `lookahead` are honoured as described under
  *Differences*; an invalid symbol or timeframe is `na` when the script passes
  `ignore_invalid_symbol` / `ignore_invalid_timeframe`, otherwise `PINE4022` / `PINE4021`. A nested call
  is `PINE4023`, a missing argument `PINE4024`.
- **`request.security_lower_tf`**: at the chart's own timeframe each chart bar is its own single intrabar, so
  it returns one-element arrays (a tuple expression a tuple of them); at a shorter timeframe there is no
  intrabar data and it returns empty arrays; a longer or malformed timeframe is `PINE4021` unless the script
  passes `ignore_invalid_timeframe`. Every other `request.*` function is `PINE9003` at compile time.

### Technical analysis and library functions

- `ta.sma/ema/rma/wma/stdev/highest/lowest/change/rising/falling/mom/roc/rsi/macd/cross*/atr/tr/bb/cum/
  barssince/pivothigh/pivotlow` (the original set) and `ta.median/range/variance/dev/swma/cmo/cci/hma/
  highestbars/lowestbars/percentrank/correlation/vwap/dmi/sar/max/min/linreg`. `ta.highest/lowest/highestbars/
  lowestbars` accept the length-only form (reading `high` / `low`). **A `ta.*` function not listed is
  `PINE4007`**; it used to evaluate to `na` silently.
- `math.sum` (a sliding sum), `array.median/mode/range/variance/stdev/percentile_nearest_rank/
  percentile_linear_interpolation/covariance` (`na` entries ignored; population figures unless
  `biased = false`), `array.sort_indices`, and `str.format_time(time, format, timezone)` (Unicode date
  patterns; an IANA zone, `UTC`, `UTC+3` or `GMT-05:30`; UTC without one).

### Drawings

- `xloc.bar_time` lines, labels and boxes: times are mapped to bar indexes when the drawing is made and
  when an x setter runs. `linefill.new/set_color/get_line1/get_line2/delete`.
  `polyline.new/delete` with `chart.point.from_index/from_time/now/new` (points are ordinary objects with
  `index`, `time` and `price` fields); `max_polylines_count` limits how many are kept.
  `table.merge_cells`, `table.clear`, `table.set_position/bgcolor/border_color/frame_color/border_width/
  frame_width`, `label.set_tooltip`.
- Every `label.style_*`: bubbles with a pointer (`label_up/down/left/right` and the four `label_lower/upper_
  left/right` corners), `label_center`, `circle`/`square`/`diamond` shapes holding the text, the glyph styles
  `cross`, `xcross`, `flag`, `triangleup/down`, `arrowup/down`, `none` and `text_outline`.
- Boxes take `text`, `text_size`, `text_color`, `text_halign/valign` and `border_style`, with `box.set_*` for each;
  lines draw `line.style_arrow_left/right/both` heads; labels keep `textalign` / `label.set_textalign`.
- Drawings made from points: `line.new(first_point, second_point, …)`, `label.new(point, …)`,
  `box.new(top_left, bottom_right, …)`, `line.set_first_point/set_second_point`, `label.set_point`,
  `box.set_top_left_point/set_bottom_right_point`. A drawing made with `na` coordinates
  (`line.new(na, na, na, na)`) exists, its getters read `na`, and it is drawn once every coordinate is known.

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
  somewhere before 5,000 bars. (Appending to histories is now done in place, which took
  about 20% off a 379-variable script; it is still interpretation-bound, and that script, the
  corpus's `xlWhYoco`, needs about two thirds of the deadline for 1,000 bars on the machine it was
  measured on, so it can report `PINE8007` on a slower one.)

## Differences from TradingView and limitations, by feature

Nothing in the first column was compared with TradingView's output unless the last column says so;
the corpus run (below) only shows that scripts compile and run.

| Feature | What this engine does | Difference or limit | Checked against TradingView |
|---|---|---|---|
| `request.security`, completed bars | On history and confirmed bars the value is the expression on the last higher-timeframe bar completed by the close of the chart bar; the chart bar that closes the bucket sees its own value. | Read from the Pine documentation, not observed. | No |
| `request.security`, realtime | The developing higher-timeframe bar, so the value repaints. | As in Pine in intent; exact tick behaviour unchecked. | No |
| `lookahead_on` | Always the developing bar; never looks ahead. | Equal to Pine for the `expr[1]` idiom. For an expression that reads the current higher-timeframe bar Pine returns the final bar on earlier chart bars of the bucket (it leaks the future); this engine does not. | No |
| `gaps_on` | `na` except on the chart bar where a new value arrives. | | No |
| Higher-timeframe data | Built by folding the chart's own bars (UTC calendar; weeks start Monday; months, quarters and years follow the calendar). | The series is only as deep as the chart's history, so a 1,000-bar chart yields few daily, weekly or monthly bars and indicators with a long warm-up show `na` for a long time. A chart with missing bars gives incomplete higher-timeframe bars. TradingView uses the provider's own higher-timeframe history. | No |
| `request.security` arguments | `symbol`, `timeframe`, `expression`, `gaps`, `lookahead`, `ignore_invalid_symbol`, `ignore_invalid_timeframe`. | `currency` and `calc_bars_count` are accepted and ignored. Only the chart's own symbol and timeframes at least as long as the chart's are served. | n/a |
| Globals inside the expression | Inputs and constants are readable. | A global *series* is not recomputed on the higher timeframe, and `myVar[1]` of a main variable is `na` there. Pine re-evaluates what the expression depends on in the other context. | No |
| `request.security_lower_tf` | One-element arrays at the chart's own timeframe; empty arrays at a shorter one. | There is no intrabar data, so scripts that rely on it (delta, intrabar volume profile) run but show nothing for it. Pine returns empty arrays only when a timeframe cannot be served; whether it accepts the chart's own timeframe (here: yes) is an assumption. | Partly (the empty result is Pine's documented behaviour for an unserved timeframe) |
| `ta.*` definitions | `median`, `range`, `variance`, `dev`, `swma`, `cmo`, `cci`, `hma`, `highestbars`, `lowestbars`, `percentrank`, `correlation`, `vwap`, `dmi`, `sar`, `linreg` follow Pine's documented formulas. | Checked against an independent implementation of the formulas on a fixed fixture, not against TradingView: ties in `highestbars`/`lowestbars` take the most recent bar, `percentrank` counts the previous `length` values at or below the current one, `vwap` restarts at UTC midnight and uses the bar's own volume, `sar` follows the equivalent Pine code in the reference manual, `linreg(source, length, offset)` is `intercept + slope * (length - 1 - offset)` of the least-squares line (the one with a numpy cross-check). `sma` and a few older functions skip `na` inside a window; the new ones return `na` if any value in it is `na`. | No (formulas only) |
| `str.format_time` | Unicode (ICU) date patterns, UTC by default. | Pine documents Java-style patterns; the common letters (`yyyy MM MMM dd HH hh mm ss SSS a EEE Z`) agree, rarer ones may not. An unknown zone name falls back to UTC. | No |
| Conditions | `na` is false in `if`, `while`, `?:`, `not`, `and`, `or`. | Matches the v6 rule that a bool is never `na`; the runtime does not know a variable's declared type, so it cannot tell a bool `na` from a number `na` and rejects only numbers and strings. | Per the v6 manual |
| `ta.*` and other missing functions | An unimplemented `ta.*` function, `table.cell_set_*` and similar are `PINE4007` at run time. | A script is not rejected at compile time for calling them, so one that only reaches them on some bars fails late. | n/a |
| Timeframe strings | `"30S"`, `"5"`/`"60"` (minutes), `"1D"`, `"2W"`, `"3M"`, bare units. | A month is 30 days for `timeframe.in_seconds` and `time(tf)` length arithmetic. `"2W"` buckets as one week, not two. | No |
| `timeframe.change(tf)` | True on the first bar of a `tf` period, and on the first bar of the chart. | The first-bar rule is an assumption. | No |
| `time(tf)`, `time_close(tf)` | Open and close of the `tf` bar containing the current bar, on the same UTC calendar. | The session and time-zone arguments are ignored. `time_tradingday` is midnight UTC of the bar's day. | No |
| `xloc.bar_time` | Times are mapped to bar indexes when a drawing is made or an x setter runs: the containing bar for a time at or before the current bar; later times forward by whole bar lengths. | Weekend and holiday gaps are not modelled. Getters return bar indexes, not times. | No |
| Variables the engine does not model | `session.*`, `syminfo.session/basecurrency/description/volumetype/mincontract`, `chart.left_visible_bar_time`, `chart.right_visible_bar_time`, `weekofyear` are `na`. | Pine always gives them a value. `syminfo.timezone` is `Etc/UTC` for crypto and `na` otherwise; `syminfo.prefix` is the exchange part of the ticker id when there is one; `syminfo.pointvalue` is 1. | No |
| Unknown dotted names | A dotted name that is not a variable, field, enum member or Pine named constant is `PINE4008`. | The constant catalogue is hand-maintained; a real Pine constant missing from it is reported as undefined. | n/a |
| `int / int` typing | Not typed as float (the checker stays silent). | The manual says division gives float; one published script (`int xc = x0 + (n - 1) * step / 2`) compiles on TradingView, so the stricter rule was wrong somewhere. The runtime still divides as float. | Only by that one script |
| User-defined types | Fields are dynamic: a declared field type is not enforced, so assigning a string to a `float` field is accepted. `copy()` is shallow. | Pine reports such assignments at compile time. | No |
| Methods | Chosen by the receiver's type as described above; the first matching definition in source order wins. | No ranking among overlapping definitions (an untyped receiver parameter matches everything, so list specific ones first). | No |
| Enums | A value is the string `"Name.member"`. | Enum values compare equal to the equivalent string, which Pine does not allow. No enum-keyed maps, no `Name.values()`. | No |
| Maps | Insertion-ordered; a whole float and the int of the same value are one key; value and key types are not enforced. `for k in map` (one variable) binds the value. | Pine requires `for [k, v] in map`. | No |
| Matrices | The operations listed above. | No element-wise arithmetic, `matrix.mult`, determinants, inverses, `matrix.reshape` or `concat`. | No |
| Polylines and linefills | Straight segments, filled when closed with a fill color; a linefill disappears with either of its lines and at most 50 are kept. | `curved = true` is not drawn curved; `chart.point.from_index` leaves `time` as `na`; the drawing code has no unit test. | No |
| Tables | `new`, `cell`, `delete`, `clear`, `merge_cells` and the `table.set_*` table-level setters. | No `table.cell_set_*` functions (`PINE4007`); merged cells are laid out by the app's own rules; the drawing code has no unit test. | No |
| Labels | Every `label.style_*`; `label.set_tooltip` / `tooltip =` and `textalign` are stored. | The chart does not show tooltips and does not apply multi-line alignment. The shape and glyph styles (`circle`, `square`, `diamond`, `cross`, `xcross`, `flag`, `triangle*`, `arrow*`) are simplified drawings with the text above, below or inside; `text_outline` is plain text. Placement is unit-tested, the drawing is not. | No |
| Boxes and lines | Box text (clipped to the box, placed by alignment), dashed box borders, arrowheads on `line.style_arrow_*`. | Text wrap, font family and formatting are accepted and ignored. The drawing code is not unit-tested (the placement and arrowhead geometry is). | No |
| Declaration | `behind_chart` (either value), `explicit_plot_zorder`, `dynamic_requests` are accepted and ignored; `max_polylines_count` limits polylines. | Drawings are always painted above the candles in the app's own order; `request.*` calls are never restricted to a "dynamic" context. `scale` is `PINE9001`. | n/a |
| Drawings with `na` coordinates | They exist and stay hidden until every coordinate is known; their getters read `na`. | Setting a coordinate to `na` later keeps the previous one instead of hiding the drawing again. | No |
| Libraries | `library()`, `export` and `import` work as described under "Libraries and `import`". | Only the libraries in the Script Manager are available: nothing is downloaded from TradingView, and a path is matched on its library name alone, so the user and version parts are accepted and ignored (two versions of one library cannot coexist). The name is the script's file name, or else the title in its `library("…")`. A library constant that builds a collection is rebuilt on every read. A compiled importer holds the library as it was at compile time: after editing a library, an importer that is already running picks the change up when it is next recompiled (saved, or its chart reloaded). | n/a |
| Limits | 500k source characters; 50k tokens and nodes; 20M instructions per bar; 10 s deadline. | Pine bounds a bar by time (about 500 ms), not by steps; the deadline is checked between bars, so one runaway bar can take several seconds first. A slow script can report `PINE8007` on a slow machine (see the corpus section). | n/a |

## Known incompatibilities

The current grammar does not yet implement the `scale` declaration
argument, built-in types such as `footprint`, `request.security` for other symbols or finer
timeframes, or real intrabar data for `request.security_lower_tf` (the engine only sees the chart's own
bars). The table above lists where implemented features differ from TradingView. Label `yloc` is treated as `yloc.price`.
Qualifier metadata types exist, but full compile-time overload/qualifier inference is not
yet complete. Stateful TA warm-up matches the documented seed approach for common data,
but missing-value and conditional-call behavior needs a larger differential corpus. Non-overlay
scripts draw in their own pane (bottom 30% of the card) with a value axis fitted to their
visible outputs; `barcolor` still recolors the price candles. The pane has no crosshair,
and overlay values do not yet join price autoscaling. Plot style/location/size
coverage is partial. Runtime byte accounting, recursion detection, and a compact bytecode
lowering pass are planned; the current executable representation is the typed AST.

A `request.*` call other than `request.security` and `request.security_lower_tf` is reported
(`PINE9003`) wherever it appears, including inside an assignment or argument. Reading a plain identifier that is
not a variable, series, or builtin raises `PINE4008` at runtime instead of silently
evaluating to a string; a dotted name is accepted only when it is a variable, a field path, an enum
member or one of Pine's named constants (`size.small`, `shape.circle`, `plot.style_line`, …), which
stand for their own names, and is `PINE4008` otherwise. An integer literal too large for an `int` is `PINE2014`. A REST refresh is
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

### Real-world script corpus

`PineCorpusTests` runs published community scripts through the engine. `tools/pine-corpus/fetch.py`
downloads open-source Pine v6 scripts in TradingView's default popularity order (`--append` adds more
without replacing what the manifest lists, because that order moves daily) into the gitignored `.pine-corpus/` directory. Their authors keep the licences, so the
sources are never committed; `DegenViewTests/PineCorpus/manifest.json` records only metadata. The test skips when
the cache is absent. Each script is compiled and run over 1,000 deterministic synthetic 1-minute bars with default inputs
(1 minute so that default timeframes of 1 to 60 minutes are not finer than the chart),
and the outcome is compared with `expectations.json`: a regression and a newly supported script both fail it.
**"Compatible" here means compiles and runs to completion; no values were compared with TradingView.**

The first 30 scripts were fetched on 2026-10-01 and 30 more later the same day (60: 40 indicators, 20 libraries).
Indicators / libraries that run or compile:

| | Indicators | Libraries |
|---|---|---|
| First run, first 30 | 1 / 20 | 0 / 10 |
| After the syntax and builtin fixes | 5 / 20 | 2 / 10 |
| After user-defined types, methods, `linefill`, table calls | 9 / 20 | 5 / 10 |
| After `request.security` (own symbol), expression subscripts, `xloc.bar_time` | 13 / 20 | 5 / 10 |
| After the 500k source limit and enums | 14 / 20 | 6 / 10 |
| After maps and methods on call results | 16 / 20 | 6 / 10 |
| After lower-timeframe requests, polylines, matrices, a 20M per-bar guard | 18 / 20 | 7 / 10 |
| The 30 added scripts, first run (before the work on them) | 8 / 20 | 6 / 10 |
| All 60 after the work on them | 37 / 40 | 15 / 20 |

The 8 that still fail are blocked by whole features (a script can have several):

| Blocker | Scripts |
|---|---|
| `request.security` for another symbol (`PINE4022`) | 3 (`UyBliO8S`, `MPypFZJS`, `tY9RZ0MY`) |
| Library `import` of a library that is not in the corpus (`PINE3040`; the imports are other authors' libraries, which the corpus does not contain) | 3 |
| `scale=` declaration argument | 1 |
| Built-in `footprint` type | 1 |
| Data-dependent runtime error (`array.get` with an `na` index inside a profile function, `UTgFqITU`) | 1, not investigated |

The corpus test takes about 40 s on the machine it was written on, most of it two scripts (`xlWhYoco`, ~7 s, and
`VEvpsGHa`); a slow machine can report `PINE8007` for those.

Corpus libraries are checked only for compiling; their exports are exercised by `PineLibraryImportTests`
against stub libraries. An indicator that runs on synthetic bars has not
necessarily reached every branch (for example the code that constructs its objects); the unit tests in
`PineUserTypeTests` cover the semantics.
