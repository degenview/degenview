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
4. Editing the script, changing an input, or a different symbol, timeframe or set of bars makes the
   profile **stale**: it stays readable but is greyed and labelled, and is never drawn on lines it no
   longer describes. *Run again* replaces it.

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
mapped onto the script's lines (their offsets belong to another text). `request.*()` is timed as the
synchronous work the engine does to answer it (the expression, the lookup); waiting for data happens
before a profile starts, because a profile runs on bars already loaded.

## How it is built

* `pine::profile` (pure; no egui):
  * `RegionTable` is built once from the compiled program: regions in source order with their start
    and **end** (the parser now records where each statement ends, because its ranges are start-anchored),
    lexical parent, and the region of the first statement of each nested block.
  * `ProfileRuntime` is the recorder. `Session::profile` is `None` unless a profile was asked for. The
    interpreter tests that one pointer in the places it already tests for a debugger, and
    the statement loop that runs without hooks does less work than it did.
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
* A run that fails, is cancelled or runs out of time keeps what it measured, labelled **partial**
  with the reason; a partial profile is never presented as complete.

## In the window

* **Toolbar** (right side, beside the debugger's bug): the stopwatch turns the profiler on and off
  (⌥⌘R, Ctrl+Alt+R elsewhere). While it is on, a chip beside it says what it is doing and does the
  obvious next thing when clicked: *Run profile*, *Profiling… 240/500* (click to cancel), *Profile
  ready* (click to profile again), *Partial profile*, *Profile stale · Run again*, *Profile failed*,
  *Waiting for chart…*. Turning it on opens the drawer's **Profile** tab.
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
* **The Profile tab**: the state, a progress bar while it runs, the summary (total time and bars,
  executions, hottest line, block and function, time in no line, the profiler's own overhead, the
  clock's resolution, how long compiling took), and the ten hottest lines by their own time; click
  one to go to it. A partial profile says so at the top. A stale one says so, is dimmed, and its rows
  do not navigate.
* The editor keeps its syntax colouring, selection, find highlights and diagnostics: the wash is
  painted under the text with the current-line band, and the column is a separate rectangle.

## Scope and what a profile belongs to

A profile is of the script **open in the Script Manager** on the **preview chart's** bars (up to the
500 candles the preview holds), with the preview's inputs, symbol and timeframe: the same scope as the
debugger. It is not the run of a script already applied to a chart, whose text, inputs and bars can be
different. Each profile records the source text, the inputs, the symbol, timeframe, number of bars and
the replay state it was produced for, and is stale as soon as any of them changes. The Script Manager
has no bar replay; there is no future to expose.

## Not verified / limitations

(Filled in as the work is verified.)
