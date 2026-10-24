# Pine Logs

`log.info()`, `log.warning()` and `log.error()` write messages a person reads while debugging a
script. They are not the app's own diagnostic log: they are script output, kept per script run.

This page describes what the engine does, what the pane shows, and what is verified.

## Language

```pine
log.info("Message")
log.warning("Close: {0}", close)
log.error("Bar {0}, volume {1,number,#.##}", bar_index, volume)
```

- One string argument is the message as it is. With more arguments the first is a format string
  filled as `str.format()` fills it (`strings::format_template`), so `{0}`, `{1,number,#.##}`,
  `na` as `NaN`, and a `{n}` with no such argument left as written all behave as in `str.format()`.
- The call returns `void`, never stops the script and never changes what it computes.
- Compile errors (`PINE3052`, `pine::log_call`): no argument, or a first argument that is a literal
  but not a string. A value computed at run time is formatted as it is.
- Calls work anywhere code runs: blocks, loops, user functions, methods, exported library
  functions (the message comes from the library's source, see below) and `request.*()` expressions.

## What a message holds

`pine::logs::LogEvent`: level, text, time, the bar's open time, the chart bar index, whether the
bar was realtime and confirmed, the library it came from (if any), the call's position in that
source (`SourceRange`, UTF-16 offset as the editor wants it), and the requested symbol and timeframe
when written inside a request. `sequence` numbers every call of a run, including ones a limit
dropped.

The call site is the compiler's range of the call, never a search of the source text.

## Historical and realtime

| | Verified against TradingView's docs | Here |
|---|---|---|
| Historical bars | one message per executed call, time = bar open time | same |
| Realtime | any tick, confirmed or not; time = system time; **not rolled back** | same |
| Rollback | messages survive, variables roll back | the sink is a `Session` field, not part of `RuntimeState` (like emitted alerts) |
| Limits | last 10,000 historical messages; realtime appended after | same; display order is historical block then realtime block |
| Restart | history kept "until the script restarts" | any rebuild starts the history over: new source, inputs, symbol or app theme, and any change of the bars that is not a plain append or a live tick. On a live chart that includes loading older history (the bars are prepended) and a tail refresh, so realtime messages written before either are lost; a replay step back or seek too |
| Strategies | | a message exists only if the script executed: unconfirmed ticks write nothing unless `calc_on_every_tick` |

Limits that are **ours**, not TradingView's: 50,000 realtime messages (the oldest tenth goes when it
is exceeded), 1,000 messages per realtime execution, 4,096 characters per message (cut with `…`).

## Requests (`request.security`)

Inferred, not verified in TradingView. Messages from the requested expression carry the symbol and
timeframe, the requested bar's open time and no chart bar index. A committed evaluation (each
requested bar once) writes. A developing evaluation writes only while the chart bar is realtime;
on a historical chart bar it is muted, because every chart bar inside one requested bar repeats it.

## Library code

A message written by library code carries the library path and a range in the library's own
source. Navigating to it needs the library's source, not the script's.

## Who reads them

Capture is off by default: `ExecutionController::with_logs()` turns it on. The chart's `PineWorker`
does; alert subscriptions, one-shot runs and the corpus do not, so they format and keep nothing.

Each `ExecutionUpdate` carries a `LogBatch`. Batches cover consecutive `from..to` sequence ranges;
a reader (`LogHistory`) that sees a batch whose `from` is not where it stopped throws away what it
held and sets `gap`, rather than showing a history with a silent hole. `reset` marks a restarted
run. Messages belong to the run, not to an output generation, so a result whose output went stale
still delivers its messages.

`ChartCard` keeps one `LogHistory` and an epoch per run (`ChartCard::pine_logs`). Hiding or removing
a script drops its run and with it its history; changing the app theme starts a new run too.

## The pane

Opened from a script's chip menu or pane menu (docked on the right of its chart, one pane per tab).
Title and close; a script selector (two copies of one script read `Name` and `Name #2`); then the
logging switch, search, start date and level filter.

- **Rows** list the messages oldest first, newest at the bottom: the time in UTC (ISO 8601 with
  milliseconds) above the text, which is gray for info, orange for a warning, red for an error.
  Text is drawn as plain text; multi-line messages keep their lines and long tokens wrap. A
  right-click copies the message, with or without its time. Only the rows on screen are laid out;
  heights are estimated, then measured as rows come into view.
- **Following.** The list sticks to the bottom while it is there; scrolling up stops it and a
  "Latest" button jumps back. Changing the script jumps to the latest message.
- **Disable logging** stops keeping the messages the script writes in realtime; the script keeps
  running, what was kept stays, and historical messages are unaffected. Turning it on again keeps
  the new ones.
- **Levels** are three checkboxes in the `…` menu. Hidden messages stay in the history; the calls
  still run.
- **Search** is case-insensitive for ASCII by default; "Aa" matches case, "ab" matches whole words
  (set off by white space or `. , : ; ' "`; a term holding one of them never matches), ".*" reads
  the query as a regular expression. Matches are highlighted. A query that does not compile is
  flagged and filters nothing.
- **Start date** takes a UTC date and time; a tag above the list shows it and removes it.
- Filters compose (level, start date, search) and never change the history. The listed rows are
  recomputed only when messages, a filter, the script or the run change; new messages are scanned
  alone.
- **Empty states**: no scripts on the chart; the "How to work with logs" steps when a script has
  written nothing; "Logging is disabled" with a button; "No matching messages" with "Clear filters".
  The copy of the last three is ours, not TradingView's.
- The level and search options are remembered per machine (`pineLogs.prefs`, never synced). The
  messages, the selected script and the pane's size and openness are not saved.

### Source code and Scroll to bar

Hovering a row shows two buttons (the same two are in its right-click menu): **Source code**
(`</>`) and **Scroll to bar**.

- **Source code** opens the Script Manager (the tab there already is, or a new one) on the saved
  script the chart's instance came from, with the caret on the `log.*()` call: the call's
  `SourceRange` start, which is already the UTF-16 offset the editor takes, never a search for the
  message text. A message written by library code opens the library (found by the name in its
  import path). A script edited since the message was written still opens, with a note that the
  line may have moved; a script that is gone, or an instance not added from a saved script, says so
  and changes nothing.
- **Scroll to bar** scrolls the chart to `bar_time_ms` (not the message's time: a realtime
  message's time is the wall clock, and a message from `request.security` carries the requested
  bar's open time, which lands on the chart bar containing it). A bar older than the candles the
  chart holds is not loaded for (older history restarts the script and clears its messages); the
  chart goes to its oldest bar and says so. See
  [Chart navigation](chart-navigation.md#scrolling-to-a-bar).

### In the Script Manager

The drawer under the preview chart has a **Logs** tab beside Inputs, Report and Problems: the same
pane (toolbar, script selector, list, filters, search, start date, the hover actions) over the
preview chart's run of the script being edited, without the pane's own title. There "Source code"
moves the caret in the editor you are in, and "Scroll to bar" moves the preview chart, which only
holds the candles it fetched. TradingView opens the pane from the editor's More menu; DegenView's
Script Manager has no such menu, so the drawer tab is its way in. The level and search options are
the same local `pineLogs.prefs` setting as the chart's pane.

### Unknown `log.*` functions and CRLF scripts

`log` has three functions, so `log.hi()` is a compile error (`PINE3051`, from `name_check`), not
only a runtime one: the editor's status says "1 error" and the Problems tab lists it. A script that
imports a library `as log` keeps that library's members.

The compiler folds each CRLF into one line feed before it reads the text, so its offsets are one
short per CRLF before them, while the editor keeps both characters. Anything that takes a compiler
offset into the editor (Problems, the underline, "Source code") converts it with
`syntax::compiled_utf16_to_char_index`. Line and column were never affected.

Pane regexes use the engine's `regex_lite` subset (the one behind `str.match`), not TradingView's:
no `\xhh`/`\uhhhh` escapes, lookarounds or named groups, and a step budget bounds the search.

## Not verified / known differences

- The format string has no `'…'` quoting: it follows `strings::format_template`, which `str.format()`
  shares.
- The `request.security` rules above, the lifecycle on hide/remove/theme change, replay behaviour,
  the rebuild triggers listed under Historical and realtime, and the extra limits.
- The editor's signature help shows one `formatString, arg0, …` row for all three functions, not
  the two overloads.
- The pane, Source code and Scroll to bar were checked through headless egui frames, unit tests and
  rendered pictures, not in a real window.
- Source code in the Script Manager offers a script edited since as a note rather than showing a
  diff, and an offset into text that changed may land on a different line.
- A formatted message whose first argument is `na` is the text `NaN`.
