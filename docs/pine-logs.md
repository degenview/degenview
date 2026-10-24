# Pine Logs

`log.info()`, `log.warning()` and `log.error()` write messages a person reads while debugging a
script. They are not the app's own diagnostic log: they are script output, kept per script run.

This page describes what the engine does and what is verified. The pane that shows the messages is
a later change.

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
| Restart | history kept "until the script restarts" | any rebuild (source, inputs, symbol, a non-prefix change of bars such as a replay seek) starts the history over |
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

## Not verified / known differences

- The format string has no `'…'` quoting: it follows `strings::format_template`, which `str.format()`
  shares.
- The `request.security` rules above, the lifecycle on hide/remove/theme change, replay behaviour
  (a replay step back or seek is a rebuild and restarts the history), and the extra limits.
- A formatted message whose first argument is `na` is the text `NaN`.
