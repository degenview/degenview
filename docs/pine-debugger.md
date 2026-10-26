# Pine debugger

A source-level debugger for the script open in the Script Manager: breakpoints, stepping, variables,
watches, the call stack, and a pause on runtime errors, driving the real engine. TradingView offers
nothing like it (its docs describe `log.*()` for debugging), so nothing here is checked against
TradingView; the Pine language semantics it relies on are the engine's own.

The engine is a tree-walking interpreter, so the debugger stops *before a statement*, never inside
an expression. Everything below follows from that.

## Using it

1. Click a line number, or press the breakpoint key with the caret on a line, to set a breakpoint.
   A line with no code (a blank, a comment, the rest of a multi-line statement, `indicator()`, a
   function header, a `type`) is refused with the reason.
2. Open the debugger panel under the editor (the bug button at the top right; starting a session
   from the keyboard opens it too). Its top row holds the controls: ▶ continue or start, ⏸ pause,
   step over, into, out, step bar, restart and stop, then a separator and what the session is doing
   ("Paused on a breakpoint at line 5"). ▶ runs the script on the preview chart's bars to the first
   breakpoint; stepping when nothing is running starts a session stopped at the first statement.
3. The panel shows the stop, and the editor scrolls to the stopped line when it is not already in
   view (every stop, steps included, not only breakpoints); ⏸ stops a running script at its next
   statement.

A session runs on a **frozen copy of the preview chart's bars**, as the preview has them at the
moment it starts (up to 500 candles). Later fetches and ticks do not reach it. Edit the script while
a session exists and the panel says the session is running the old text; Restart runs the new one.

| | macOS | Windows / Linux |
|---|---|---|
| Start / Continue | ⌃⌘Y | F5 |
| Pause | ⌃⌘P | F6 |
| Step Over | F6 | F10 |
| Step Into | F7 | F11 |
| Step Out | F8 | Shift+F11 |
| Step Bar | ⌃⌘B | F7 |
| Stop | ⌘. | Shift+F5 |
| Restart | ⇧⌘R | Ctrl+Shift+F5 |
| Toggle breakpoint | F9 | F9 |

F5 used to open completion as well; it no longer does (Ctrl+Space remains). On a Mac the function
keys may need Fn.

## How it runs

```
editor text ─▶ compile ─▶ SourceMap (statement offsets, lines, scopes)
                              │
 preview bars (frozen) ───────┼─▶ DebugSession thread ─▶ ExecutionController ─▶ Session ─▶ hooks
                              │        ▲  │ events (bounded)                         │
   UI: toolbar, gutter, panel ┘  commands/requests │                         DebugRuntime
```

* `Session` (the interpreter) has an `Option<Box<DebugRuntime>>`. Only `attach_debugger` sets it.
  `Session::run` (every statement list), `invoke` (a user function call), `execute_bar` and the
  `request.security` evaluator each test that option and call a `#[cold] #[inline(never)]` helper.
  There is no hook in `eval` and none in the instruction counter. A session without a debugger runs
  the code it always did, plus those tests. See [Performance](#performance).
* The runtime takes itself out of the session for the length of a hook, so it can be handed
  `&mut Session`: no aliasing, no `unsafe`.
* A `DebugSession` is its own `std::thread` with its **own** `ExecutionController`, built the way the
  chart's worker builds one, so an open last bar runs as a realtime tick with rollback. It shares
  nothing with the preview or any chart: no alert routing, no paper trading, no broker, no outputs.
  `ExecutionUpdate.alerts` and `output()` are dropped; the only things that come back are stops, log
  batches and the outcome.
* The thread blocks, with a 40 ms poll, inside the hook while stopped. It holds no lock. UI → thread
  is a bounded `sync_channel` of requests; thread → UI is a bounded one of events. Every request
  carries the generation of the stop it was made for, so one made against a stop the script has left
  is answered `Stale` and a resume can never skip a stop. Values live on the thread's stack, so the
  thread answers inspection and evaluation requests while stopped.
* Dropping a session (switching scripts, closing the tab or window, quitting) sets its cancellation
  flag and closes its channels; the thread ends at its next safepoint or within a poll.
* Time spent stopped is added back to the script's deadline; the instruction and call-depth limits
  are untouched, and a script that really runs away still ends with `PINE8007`/`PINE8004`.

States and the commands each accepts are one pure state machine (`pine::debug::DebugState`); the
toolbar enables a button exactly when the session would accept the command.

## Source mapping and stepping

A statement is identified by the UTF-16 offset where the parser anchored it (ranges are
start-anchored; the language has no `;`). Lines are never identities. The `SourceMap` lists the
stoppable statements in order, with line and column, the user function each belongs to, and the names
visible at each.

Stops and steps work on *frame depth* and bar boundaries, never on line numbers:

| Command | Stops at |
|---|---|
| Continue | the next breakpoint, error, pause request, requested bar or the end |
| Step Into | the next statement, entering user functions |
| Step Over | the next statement at the current depth or shallower |
| Step Out | the next statement shallower than the current depth: the caller's *next statement* (not mid-line) |
| Step Bar | the first statement of the next bar's execution |
| Pause | the next statement |

Notes: stepping out of the script frame runs on like Continue. A loop stops once per iteration in its
body and once for its header. A multi-line statement is one stop, at its first line. A breakpoint on
a statement reached during a step wins over the step. Library functions are *stepped over*: they
appear in the stack as `name() (library)` but have no stops and no breakpoints (not verified beyond
that). A realtime re-execution of the same bar counts as a bar for Step Bar.

The call stack is the debugger's own: the interpreter keeps none, so `invoke` reports each call, with
the caller's values of the names the function declares (the interpreter removes them from its flat
variable table for the length of the call). A `request.security()` expression is a frame of its own,
`request.security()  SYMBOL / 5m`, between the expression's frames and the chart's: the frames above
it read the expression's variables, the one below reads the chart's back from where the interpreter
set them aside, anything further out (a request inside a request) is unavailable and says so.
Evaluating a watch outside the expression's boundary is refused.

## Breakpoints

A `BreakpointSpec` is a line, an on/off switch, an optional condition, a hit rule and an optional
logpoint message. They are compiled once against the source map; a condition or message that does not
parse or is not allowed is reported on the breakpoint, and a line with no code is `Unresolved`.

* **Hit counts** count each time the statement is reached with the condition true, across bars and
  calls, from the start of the session. Rules: always, the nth hit, every nth, after the nth; the
  breakpoint's editor picks one from a drop-down, takes the number beside it and says in a sentence what
  it will do.
  Restarting counts from zero. A live edit keeps the counts of breakpoints that keep their id.
* **A condition that cannot be evaluated** (an out-of-range `array.get`) stops the script and says
  why, rather than silently never firing. On a logpoint it writes a warning instead.
* **Logpoints** render `{expression}` with the same evaluator and write an ordinary Pine Logs event
  at the statement's range with the bar's context, then go on. They share the log sink's limits
  (10,000 historical messages), so a long run cannot grow without bound. The Logs tab shows the
  session's messages (the script's own `log.*()` calls too) while a session exists.
* **Breakpoints are consulted on every visit**, so a logpoint still writes and a hit still counts when
  a pause or bar break stops on the same statement.
* **Stop at bar** stops at the first statement of one bar, once. It is not a line breakpoint. One
  line of controls chooses the bar, and each shows what the others chose: **Select bar**, which waits
  for a click on a candle of the preview chart (the chart's "click a candle" mode, as the replay uses
  it; Esc or the button again cancels); a small field for the bar's **index** (0 to the last bar;
  Enter or leaving the field arms it, emptying it disarms); and a **date** and a **time of day**
  (UTC) picked from drop-downs that list only the days and times the chart has a bar for. A chart of
  one bar a day has no time picker. Choosing another date keeps the time of day when that date has a
  bar at it. The armed stop is shown as "Stops at bar 31 · 2026-10-24 12:00 UTC" with a button to
  take it back, or says why it
  cannot happen (a bar past the end, or a time after the last bar). The stop is kept as a UTC time
  or an index; a time stops at the first bar opening at or after it. The lists are the preview's
  bars, and a session runs on a copy taken when it started, so a candle newer than the copy never
  stops it.
* Breakpoints can be added, removed and edited while the script runs.

Persistence: per script, on this Mac only (`scriptDebug.state`, a local setting). They follow edits
through the same line diff the editor's change gutter uses; one whose line was *deleted* is kept,
marked unresolved, never installed, and never moved onto whatever code took its place. Deleting a
script forgets its state.

## Inspecting a stopped script

The interpreter's variable table is flat and keeps names after their block ends and from earlier
bars, so what a stop shows is decided from the syntax: the source map gives each statement the chain of
names visible to it (parameters, loop counters, block locals, globals declared before a function).
Values are read from the table with the caller's parked values laid over it for a caller frame. A
name in scope whose declaration has not run on this frame this bar reads "not yet assigned on this
bar" instead of last bar's value. Groups: Local, Global, Persistent (`var`/`varip`), Built-in.

Containers (arrays, maps, matrices, objects, tuples) and series history expand lazily, a page of up
to 200 rows at a time, through references that name a path (frame → name → element → field) and the
stop they belong to: a reference outlives nothing, and a collection that shrank answers "no longer
available". Nothing is copied wholesale; formatting never recurses. Open rows stay open from stop to
stop by their path of names. A row whose value changed since the last inspected stop is marked.
A scalar variable expands into its **series history**, one row per earlier bar, named `[n]` as in
Pine (`close[1]`) and labelled "this bar", "1 bar ago", "n bars ago" where the type would be: they
are bars, not elements of an array. An **input** has no history (it is the same on every bar), and
a series that held one value on all of the last 5,000 earlier bars is shown as that value once with a
"same value" row instead of a column of identical ones. The history of a function's local is not
offered (its series is keyed by call site).

## Watches, conditions, logpoints, hover and the console

One expression service. Text is parsed by the Pine parser (call sites numbered from 2^40, far above any
script's) and accepted only if every part is a read:

* allowed: literals, names in scope, operators, `?:`, `x[n]` **on a plain name**, fields, tuples,
  `math.*` (except `random`), `str.*`, `color.*`, `na`/`nz`/`int`/`float`/`bool`, the read-only
  `array.*`/`map.*`/`matrix.*` functions and methods (`get`, `size`, `first`, `last`, `includes`,
  `indexof`, `avg`, `min`, `max`, `sum`, …).
* refused, with the reason: `ta.*` and `fixnan` (they keep a window per call site), indexing an
  expression (`(a+b)[1]` keeps its history in a slot named after its source offset, which would
  collide with the script's own), user functions, mutators and allocators (`push`, `put`, `slice`,
  `copy`, `keys`), `plot*`, drawings, `strategy.*`, `alert*`, `log.*`, `request.*`, blocks.

A watch is added with Enter or the **Add** button beside the field. Its edit and remove buttons appear
on hover in a strip kept at the row's right edge, beside the type rather than over it.

Evaluation also restores the instruction counter, any handle it allocated, and the variables it put
back for a caller frame. Tests evaluate watches, take variables and expand values at every statement
of six workloads and of a script with requests and compare plots and strategy reports with a clean
run (identical), and compare the whole `RuntimeState` before and after an evaluation.

Not supported: `ta.*` in a watch or condition (compute it into a variable in the script and watch
that), and anything stateful.

## Pine specifics

* **Historical bars** step as above. **Realtime**: an unconfirmed last bar runs as a tick over a
  clone of the committed state; stepping through it works, `var` rolls back and `varip` keeps
  counting, the bar context says Realtime / Unconfirmed. A session holds a frozen snapshot, so it
  runs the last bar once as a tick, then ends; it does not follow later ticks.
* **Synthetic chart types** (Renko, Heikin Ashi, …): the debugger is fed whatever bars it is given.
  The Script Manager's preview is always plain candles, so a synthetic series is exercised only by
  the engine's tests, not through the window.
* **Strategies** run on the session's own broker; nothing reaches the paper account, the chart or
  any alert route.
* **Runtime errors** stop *at* the failing statement with the stack and variables intact, once per
  failure however far it unwinds. The stop is read-only: Continue and the steps are disabled; Stop
  and Restart remain, and the outcome is `Failed` with the diagnostic. Resource errors
  (`PINE8004`/`PINE8007`) have no location of their own; the failing statement is used.

## The panel

The controls row (the buttons, a separator, the status line, then a note, the restart-on-new-code
button and close; the status shortens to a dot with the words on hover, and leaves when the buttons
use the whole width); the bar context (bar index, symbol, timeframe,
historical/realtime, confirmed, OHLCV, the request when inside one); Variables with a filter (the
same flat field as the watchlist's); Watches (add, edit in place, remove, expand);
Call Stack (click a frame to show its variables and watches and mark its line); Breakpoints (switch,
condition, hit rule, logpoint message, hit count, why one is unresolved or invalid, remove, enable or
disable all, stop at a bar); Console (evaluate, history). Wide windows put the variables left and
the lists right; narrow ones stack. Sizes and open sections persist in the Script Manager's layout.

In the editor: the breakpoint column beside the line numbers (filled dot, conditional dot with `=`,
logpoint diamond, hollow ring for disabled or unverified), an arrow and band on the stopped line
(outlined arrow and fainter band for a caller's line, red for a failure), the values of the line's
names after its text, and a tooltip with a name's value on hover. Picking a row of the variables gives
the list the keyboard: Up/Down, Home/End, Right opens (or steps into) a row, Left closes it (or goes to
its parent), Return toggles; the other lists are reached with the mouse and Tab.

## Performance

Measured with `tools/pine-bench.sh f4fab25` (the commit that added the harness, before any engine
change) on one Apple-silicon Mac, `--release`, twelve interleaved rounds of five runs each, the median
of each round's median. `checksum same` means both builds computed the same plots and trades.

**Normal execution (no debugger attached), before → after:**

| workload | base ms | head ms | change | rounds base / head (ms) |
|---|---:|---:|---:|---|
| simple_indicator (5,000 bars) | 13.90 | 13.98 | +0.56% | 13.65–24.77 / 13.46–25.64 |
| straight_line_800 (800 statements × 800 bars) | 100.48 | 99.41 | −1.06% | 97.50–235.67 / 94.12–228.54 |
| many_ta | 28.41 | 28.46 | +0.17% | 27.11–52.36 / 27.34–73.02 |
| long_history (20,000 bars) | 49.78 | 49.58 | −0.40% | 48.00–90.92 / 47.03–104.37 |
| containers (array, map, matrix) | 95.35 | 95.17 | −0.19% | 92.60–164.03 / 91.95–277.55 |
| nested_functions | 24.78 | 24.73 | −0.22% | 23.98–58.70 / 23.74–71.04 |
| request.security | 12.92 | 12.90 | −0.16% | 12.65–23.15 / 12.47–44.79 |
| strategy backtest | 17.15 | 16.98 | −1.03% | 16.96–39.13 / 16.49–35.13 |
| four scripts at once | 18.54 | 18.31 | −1.23% | 17.13–37.51 / 17.56–29.45 |
| realtime ticks (300 bars × 3) | 136.32 | 133.11 | −2.35% | 131.48–318.92 / 130.14–298.71 |
| compile (800 statements) | 1.40 | 1.38 | −1.50% | 1.32–1.53 / 1.29–2.74 |

The medians sit between −2.4% and +0.6%. The same script built twice from the same commit (an A/A run,
ten rounds) differs by −2.5% to +0.9% between its two binaries, so run-to-run and layout noise is about
±2.5% and **this benchmark cannot resolve the 1% target**: the result is "indistinguishable from the
baseline", not "proven under 1%". The wide ranges in the slowest rounds above are from a release build
that ran in the background during part of the run. Two earlier runs against the same baseline, at the
commit that added the first hooks (eight rounds) and at the commit that completed the engine side (ten
rounds), gave −4.5% to +1.2% and −1.9% to +3.6%: differences on both sides of zero in every run, and no
workload was slower in all three runs.

**Allocations** (`--features alloc-count`, one run each): allocation counts and bytes requested are
**identical to the byte** for all eleven workloads, so the hooks add no allocation to a run without a
debugger. The peak differs by a constant ~1.5 MB for every workload (and by 0.5 KB for `compile_800`),
which is the benchmark binary holding the debug workloads' bars, not the engine.

**Why the cost is low.** The statement loop gained `let debugging = self.debug.is_some()` and, behind
it, two calls into `#[cold] #[inline(never)]` functions; `invoke`, the `request.security` evaluator and
`execute_bar` gained one such test each; `eval`, `budget` and every per-expression path gained nothing.
The `Option<Box<_>>` is a pointer-sized null test the CPU predicts perfectly. The alternative that was
considered and not needed is moving the test into `budget()`'s existing counter path.

**Debug mode** (the same run; twins are named in `src/pine/bench_tests.rs`):

| mode | ms | against |
|---|---:|---|
| attached, nothing to stop at (simple, 5,000 bars) | 14.59 | `simple_indicator` 13.98: +4% |
| a breakpoint whose condition never holds, evaluated every pass | 14.92 | +7% |
| attached, 800 statements × 800 bars | 130.43 | `straight_line_800` 99.41: +31%, about 48 ns per statement |
| stepping through every statement, variables and a watch read at each stop (simple, 150 bars, ~900 stops) | 5.43 | `plain_simple_150` 0.43: ~5.5 µs per stop |
| the same on nested functions (150 bars) | 15.36 | `plain_functions_150` 0.78 |
| building the source map for 800 statements | 0.06 | compiling them takes 1.38 |

Debug mode allocates about 177 times per stop (the variable list and the watch), nothing per statement
that does not stop: `debug_attached_simple` makes 5,372 more allocations than `simple_indicator`
over 5,000 bars, which is the harness compiling the script a second time for the map. The map for 800
statements is 292 KB in 3,220 allocations (the compiled program is 4.2 MB). Every debug run produced
the same checksum as its plain twin.

Latency: `cargo test --release --lib pine::debug::stress_tests::latency -- --ignored --nocapture`
prints the time from a Pause request to the stop and from a step to the next stop on a running loop.

## Tests

`cargo test --lib pine::debug` (engine: stepping matrix, breakpoints, inspection, watches, Pine
specifics, realtime, sessions, stress) and `script_manager::debug_*` (through the window).
`tools/pine-bench.sh <base> [head]` is the A/B benchmark; `cargo test --release --lib
pine::bench_tests -- --ignored --nocapture` prints one workload per line, with
`--features alloc-count` adding allocations and peak memory. Pictures: `DEGENVIEW_GALLERY_DIR=… cargo
test --lib script_manager::debug_gallery -- --ignored`.

## Not verified / limitations

* Nothing is checked against TradingView, which has no debugger.
* The window was exercised by headless frames, rendered pictures and a pass over a release build
  (see the PR for exactly what was done); there has been no wider manual pass. The scroll to a stop
  is tested headless with the scroll animation off; the animated scroll was not watched.
* A session is a frozen copy of the preview's bars; it cannot follow live ticks, and a chart-applied
  instance cannot be debugged (only the Script Manager's script).
* Library code is stepped over; breakpoints in a library or in code reached only through one are not
  supported.
* `ta.*`, `fixnan` and other per-call-site state cannot appear in an expression.
* A request inside a request: only the nearest enclosing context's variables are readable.
* Lines in the editor are split on `\n`; a lone CR or U+2028/2029 in a script is a line break to the
  compiler but not to the editor, so line numbers after one may disagree.
* No hot reload and no stepping backwards: Restart runs the new text from the first bar.
* Only the variables list is navigable with the arrow keys.
