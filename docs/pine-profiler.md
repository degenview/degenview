# Pine profiler

Shows where a script's execution time goes: a bar and a share beside each line of the Script
Manager's editor, the cost of whole loops and branches as code blocks, and a ranked list of the
hottest regions. The engine measures; the editor only draws what it is handed.

TradingView documents the *presentation* of its profiler (bars and percentages beside significant
lines, flames on the top three regions, a tooltip with time and executions, code-block ranges, the
line/block distinction) but not how it measures. The measurement model below is this project's own and
is described exactly, so a number on screen has one meaning.

## Using it

1. Open a script in the Script Manager and click the profiler button in the toolbar (the stopwatch,
   next to the debugger's). The profiler is **off** by default and costs nothing while off.
2. Turning it on runs the script once on the preview chart's bars, on a thread of its own. The button
   reads *Profiling…* with a progress count, then *Profile ready*.
3. The editor gains a narrow column beside the line numbers: a bar per measured line, its share of the
   script's time, a flame on the three hottest regions. Hover a line for the details; hover its
   column to highlight the code block it covers. The panel under the editor ranks the hottest regions;
   click one to go to it.
4. A profile belongs to the text, inputs, chart and bars it was made for. Edit the script and it is
   **stale**: the column goes empty (its line numbers describe other text), and the tab keeps the old
   figures, dimmed, with a banner. Change an input, the symbol or timeframe, or let the live chart get
   a new candle, and it is stale too, but the text is the same, so the marks stay on the column,
   faded, and rows of the list still go to their lines. *Run again* replaces it.

## What is measured

The interpreter walks the syntax tree, so the unit of measurement is a **region**: one statement (a
declaration, an assignment, an expression statement, a loop, an `if`, a `switch`, `break`/`continue`)
or a user-defined function. A region is identified by the UTF-16 offset where the parser anchored it,
the same identity the debugger uses; a line is only how a region is shown. Two statements on one line
(`a = 1, b = 2`) are two regions that the line adds together.

| Metric | Meaning |
|---|---|
| **Executions** | How many times the region ran: a statement each time the interpreter reached it, a function each call. A loop statement counts its *entries*; its iterations are counted separately (below). A branch's body counts only when it ran. |
| **Time** (inclusive) | Wall time from start to end of each execution, summed, including every region nested inside it. |
| **Line time** (exclusive) | Inclusive time minus the inclusive time of the regions that ran inside it. The loop header's own work, not its body's. |
| **Slowest** | The longest single execution (inclusive). |
| **Iterations** | For a loop, the executions of the first statement in its body (`continue` included). For an `if`/`switch`, the same per branch. Derived, so the loop pays nothing to be counted. |

Percentages divide by the **script total**: the summed wall time of every bar execution. Time (and its
share) can add to more than 100% down a column because nested regions are counted in their parents;
**line time cannot**: line times, plus the *not attributed* row, add to the script total exactly.

* *Not attributed*: time inside a bar that no region covers (setting the bar up, committing
  histories, the strategy broker's fills, `indicator()` itself).
* *Profiler overhead (estimated)*: every measured execution costs two clock reads and some
  bookkeeping. That cost is measured once per profile on this machine (an empty region, many times,
  median) and multiplied by the number of measured executions. It is **reported, never subtracted**:
  a region's time is what the clock saw, and the profiled run is slower than a normal one.
* Compilation is not part of any of this; the profile states how long it took separately.

Recursion: a region already on the call stack adds its elapsed time to *line time* but not again to
*time*, so a recursive function's time is never more than the time it took.

What is **not** timed separately: expressions. A built-in (`ta.sma`, `array.sort`), a ternary or an
operator is part of the statement that contains it, as in TradingView where a wrapped line is
attributed to the line it starts on. Split a statement onto several lines to learn which part costs.
Time inside a library's functions is part of the statement that called them; library lines are never
mapped onto the script's lines (their offsets belong to another text). `request.security()` reading another symbol may fetch its candles over the network, blocking: that time
is measured around the provider and kept out of every region and out of the total (the clock the
recorder reads is the real one less the time spent waiting), and shown as *Waiting for data*. What the
script does with the candles afterwards is timed like anything else.

## How it is built

* `pine::profile` (pure; no egui):
  * `RegionTable` is built once from the compiled program: regions in source order with their start
    and **end** (the parser now records where each statement ends, because its ranges are start-anchored),
    lexical parent, and the region of the first statement of each nested block.
  * `ProfileRuntime` is the recorder. A `Session` carries `hooks: Option<Box<Hooks>>`, a debugger or a
    profiler and never both; it is `None` unless `attach_debugger` or `attach_profiler` ran. The
    statement loop tests that one pointer once per statement, as it did for the debugger alone, and the
    hooks (`#[cold] #[inline(never)]`) do the rest. `invoke` and `execute` carry the only other hooks.
    Every start has exactly one end, including on the error path; code that is not timed (library
    functions, a function's declaration) starts an *untimed* frame that costs no clock read.
  * A stack of frames gives properly nested timing: on exit a frame's elapsed time is added to its
    parent's *children* total, so exclusive time needs no estimation and nothing is counted twice.
    The stack is closed on the error path and reset every bar.
  * Counters are aggregates in one preallocated array (`count`, inclusive, exclusive, slowest),
    sized by the program. Nothing is stored per execution, so memory does not grow with bars.
  * The clock is a trait: `Instant` in production, a scripted clock in tests, so the invariants are
    asserted exactly rather than within a tolerance.
* A **profile session** is a thread with its own `ExecutionController` over a frozen copy of the
  preview's bars, the way a debugging session is. It shares nothing with the preview or a chart: no
  alerts, no paper trading, no orders reach anything outside it, and outputs are dropped. Cancelling
  sets the controller's cancellation flag; dropping the session does the same, so closing the tab,
  switching scripts or quitting ends the thread.
* The deadline: a profiled run is slower than a normal one, so each bar extends the deadline by its
  estimated overhead; a script that would have finished in time normally finishes in time profiled.
  The instruction and call-depth limits are untouched.
* A run that is cancelled or runs out of time keeps what it measured, labelled **partial** with the
  reason; a partial profile is never presented as complete. A run that **fails** has no profile: what
  it measured leaves out the failing bar and everything after it, so the Profile tab says "Profile
  failed: the script failed after N bars: …", nothing is drawn beside the code, and a profile of the
  text before is dropped too. The engine still returns the partial report
  (`Completeness::Partial(Interruption::Failed)`); `Profiler::finish` turns it into a failure.

## In the window

* **Toolbar** (right side, between the debugger's bug and the preview chart's buttons): the stopwatch
  turns the profiler on and off (⌥⌘R, Ctrl+Alt+R elsewhere). It is blue while the profiler is on,
  amber when the profile is partial or out of date, red when it failed, and its tooltip says which.
  Turning it on opens the drawer's **Profile** tab.
* **A failure is also a problem.** A script that compiles and then fails (an unknown function, an
  index out of range) ends the profile with no data ("the script failed after N bars: …"), and the
  same error is listed in the Problems tab, counted on its badge and in the chip under the editor, and
  underlined at the failing call, exactly as the chart's own run reports it (`script_manager::problems`).
  Only if the profile is of the text on screen; a cancelled or timed-out run is not a failure.
* **The column** beside the line numbers, present only while the profiler is on, so the text moves
  once, when it is turned on, and never when a profile arrives. Each measured line has a bar, the
  line's share of the script's time as a number, and, for the three hottest lines (if at least four
  lines are marked), a flame. The bar is the line's *time* (itself and everything it runs) on one
  scale for the whole profile: the longest bar is the line that took longest. Its colour is the
  line's *own* time relative to the line that spent the most itself, from slate to orange, so a
  loop's header is long and cool when its body does the work and the body's lines are the warm ones.
  A line under 0.1% of the script's time draws nothing but can still be hovered.
* **Hover** a line in the column for its figures (time with share and total, line time when it
  differs, executions, iterations or branches taken, average and slowest, and for a function where it
  is called from); the code block the line covers is washed in the editor. A time per execution
  shorter than the clock's resolution is not averaged; the tooltip says so.
* **The Profile tab** is laid out like the Logs tab beside it: a toolbar row (a field that says what
  the profiler is doing, with a dot coloured by state and the chart it ran on, and one button for the
  next thing: run again, cancel, try again), a hairline, then the body. Off, it introduces the
  profiler in three steps with a picture of the column and a *Start profiling* button. Running, it
  shows progress and *Cancel*. With a result: the three totals (time, bars, executions), each with a tinted icon chip (clock, bars, repeat) that a tile under 120pt wide leaves out so its label stays whole; the ten
  hottest lines by their own time (click one to go to it); and two cards of figures, *Where the time
  goes* (hottest line, block and function, each a click away from its line) and *How it was
  measured* (what is in no line, waiting for data, the profiler's overhead, the clock's resolution,
  compile time), each with a note saying what the figure is of. A partial profile says so at the top.
  A stale one says so and is dimmed; its rows go to their lines only while the editor still holds the
  text that was profiled. All text in the tab is the secondary label colour or darker, so it reads in
  both appearances. Where the drawer is wide, the introduction puts its picture beside the steps and
  the hottest lines stand beside the two cards of figures; the body scrolls instead of being cut off
  in a short drawer.
* The editor keeps its syntax colouring, selection, find highlights and diagnostics: the wash is
  painted under the text with the current-line band, and the column is a separate rectangle.

## The AI assistant's profile

The Script Manager's assistant can call `profile` on its own draft (`script_manager::ai_profile`, see
[AI assistant](ai-assistant.md)). It uses the same engine, `ProfileSession`, on a frozen copy of the
preview chart's bars, and turns the report into text with `ProfileMarks`. It is a session of its own: it
does not use the editor's profiler state, so it never draws a column or fills the Profile tab, and a
profile the user ran stays as it was. A profile of the assistant's draft is of text that is not in the
editor, so showing it there would mislabel it.

## Scope and what a profile belongs to

A profile is of the script **open in the Script Manager** on the **preview chart's** bars (up to the
500 candles the preview holds), with the preview's inputs, symbol and timeframe: the same scope as the
debugger. It is not the run of a script already applied to a chart, whose text, inputs and bars can be
different. Each profile records the source text, the inputs, the symbol, timeframe, number of bars and
the replay state it was produced for, and is stale as soon as any of them changes. The Script Manager
has no bar replay; there is no future to expose.

## Performance

Measured with `tools/pine-bench.sh b49f53c <head>` (the commit before any of this) on one Apple-silicon
Mac, `--release`, twelve to fourteen interleaved rounds of five to seven runs, the median of each
round's median. Head must be a commit, not the working tree: the script builds the tree it is given.

**The profiler off (a session with no hooks).** The plain statement loop is the loop it was: one
`hooks.is_some()` per statement, the test the debugger already made. The first design, a test once per
block and a second loop for hooked sessions, measured 1–2% *slower* on statement-heavy workloads, so
it was dropped. The `invoke` hook was then given the shape the debugger's had (a test before and a
test after the call, the same arguments), because call-heavy workloads had measured +1.7…+3.1% in five
runs. The final build against the base:

| workload (quiet machine, 14 rounds) | base ms | head ms | change |
|---|---:|---:|---:|
| simple_indicator | 13.53 | 13.31 | −1.64% |
| straight_line_800 | 98.34 | 97.87 | −0.48% |
| many_ta | 27.42 | 27.82 | +1.44% |
| long_history | 47.94 | 48.99 | +2.19% |
| containers | 94.24 | 94.56 | +0.34% |
| nested_functions | 24.01 | 24.74 | +3.06% |
| strategy | 16.59 | 16.91 | +1.92% |

Two builds that differ only by a dummy function added to `session.rs` move the same workloads between
−1.4% and +2.4% against the base, and base against base between −0.5% and +0.9%. So this benchmark
cannot show that the disabled profiler costs under 1%. The hot-loop shape is the base's (one pointer
test per statement), and every plain workload stayed inside ±3%, but **call-heavy scripts
(`nested_functions`, `plain_functions_150`) measured up to about 3% slower in several runs**, and −1.8%
and −1.0% in two later ones; if that is a real cost it is at most a few nanoseconds per user-function
call, and it is not resolved here. The later run with allocation
counts (`PINE_BENCH_FEATURES=alloc-count`, a noisier machine) agrees: plain workloads +1.6% on
average, every one inside the null-change band.

**Allocations** are the same to within a handful: a plain run allocates 3–11 times more than the base
over a whole benchmark (the parser keeps statement ends: 3 allocations and ~600 bytes per compile),
and nothing per statement. Peak memory is +2 MB for every workload, the benchmark binary holding the
profiled workloads' bars.

**The profiler on** (the same run; each against its plain twin):

| mode | plain ms | profiled ms | cost |
|---|---:|---:|---:|
| simple (5,000 bars, ~8 statements a bar) | 14.93 | 18.05 | +21% |
| straight line (800 statements × 800 bars) | 122.69 | 176.31 | +44%, 84 ns per statement |
| nested functions | 27.75 | 35.57 | +28% |
| nested loops over an array | 851.69 | 1265.13 | +49% |
| building the region table (800 statements) | | 0.07 | compiling them takes 1.51 |

Every profiled run produced the same checksum (plots, trades) as its plain twin. A profile's memory is
one 40-byte counter per region plus one per call site: it does not grow with bars. The measured cost of
timing one region on this machine is about 115 ns (the summary says so), the clock's resolution 41 ns.
The run's own slowdown is in the table above; nothing is subtracted from any figure.

## Tests

* `cargo test --lib pine::profile`: counts exactly (statements, loops and their iterations, nested
  loops, `while` with `break` and `continue`, branches taken, `switch` arms, two statements on a line,
  multi-line statements, user functions from several call sites, a call in a loop, library code),
  time exactly on a stepping clock (inclusive, exclusive, the bar's total, what no line covers,
  recursion, an open region closed by the end of a bar, a refused bar), the same outputs, trades and
  alerts with and without the recorder across seven workloads including a strategy and
  `request.security`, a failing script failing the same way, and sessions (own thread, a script that
  does not compile, no bars, cancelling keeps a labelled partial profile, a failing script engine-side
  keeps what ran before it (the window then shows none of it), dropping a session ends its thread, two sessions are independent).
* `cargo test --lib script_manager::profile` (and the editor's tests): the words (units, percentages,
  separators), bars, heat, flames, tooltips, the ranked list and summary, partial and stale profiles,
  and through the whole window: off by default and hashing nothing, turning on, editing, inputs, bars
  and scripts making a profile stale, errors and libraries, cancelling, turning it off ending the
  thread, two windows. The editor's column moves the text once and washes the hovered block.
* `cargo test --lib pine::bench_tests -- --ignored` prints one workload per line, including the
  `profiled_*` twins.
* Pictures: `DEGENVIEW_GALLERY_DIR=… cargo test --lib script_manager::profiler_gallery -- --ignored`
  (off, ready, hovering, stale; wide and narrow; dark and light).

## Not verified / limitations

* **Nothing is checked against TradingView's numbers**, which are not documented. Only its
  presentation was followed.
* **A profile is of the open script on the preview chart's bars** (up to 500 candles), not of a
  script applied to a chart. The Script Manager has no bar replay, so none is handled. A profile of a
  few hundred bars runs for milliseconds: shares are meaningful, absolute times are of that run, and a
  second run differs (TradingView's own documentation shows the same spread). Run it again and compare.
* **Measurement is per statement.** Expressions inside one are not separated, and a line that costs
  less than the clock can tell apart (41 ns here) is flagged, not averaged. The timer's own cost
  (~115 ns here) is inside every figure; for statements of a few nanoseconds the profile overstates
  what a normal run spends.
* **Library code** is part of the statement that called it; library lines are never shown. Libraries
  themselves are not profiled.
* **Realtime:** the preview's last bar, if still forming, runs as one tick and the run ends. The
  profile does not follow later ticks; run it again.
* The deadline is extended by the estimated overhead of each bar; a script close to the 10 s limit may
  still time out, and then the profile is partial and says so.
* **Not exercised in a real window.** The window was run headlessly and its pictures looked at, not
  driven by hand. In particular tooltip placement near the window's edge and behaviour at other UI
  scales are unverified.
* Lines in the editor are split on `\n`; a lone CR or U+2028/2029 in a script is a line break to the
  compiler but not to the editor, so line numbers after one may disagree (as in the debugger).
* The timer's cost is stated twice because it is two things: ~115 ns is what timing one *region*
  costs (measured on an empty region), and the ~84 ns per statement in the table is the whole
  slowdown of a profiled run of one-line statements, which also pays for the stack and the counters
  and loses a little to the extra code in the loop.
