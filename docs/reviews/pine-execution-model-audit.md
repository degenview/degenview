# Pine execution-model audit

Date: 2026-11-07  
Scope: `src/pine/` (interpreter, runtime state, broker, execution controller) against TradingView's
[Execution model](https://www.tradingview.com/pine-script-docs/language/execution-model/) page.  
Review type: static review by reading the code; **nothing was run**. Each finding says how far it was
checked. No implementation changes in this document's PR.

## Method

The documentation page was read in full and reduced to 59 explicit rules (bar-by-bar execution, history
and the `[]` operator, realtime bars and rollback, `var`/`varip`, scopes, historical buffers, strategy
re-execution, caching and reload). Three independent code audits then traced:

- **A**: the bar loop, commit/rollback, `var`/`varip`, `barstate.*`, `ExecutionController`.
- **B**: time series, `[]`, local scopes, function parameters, `ta.*` state, history buffers, warnings.
- **C**: strategy execution, declaration parameters, broker, alerts, errors, reload.

*Verified-by* column: **re-read** = I read the cited lines after the audit and the claim holds;
**audit** = reported by an audit, not re-read; **needs TV** = the engine's behaviour is clear but what
TradingView does is not documented on this page and must be checked on a chart before changing it.

The page does not cover `process_orders_on_close`, a per-bar `barstate.*` table, loop or call limits,
`last_bar_index`, or `request.*` consistency rules. `calc_on_every_history_tick` is mentioned there as a
setting; it is not a `strategy()` parameter the engine accepts (`PINE9001`), and whether TradingView has
a declaration parameter by that name was not confirmed.

## Summary

The core model is sound: one execution per historical bar, a committed/working copy of `RuntimeState`,
rollback on every unconfirmed tick, commit only on the closing tick, `log.*` outside the rollback, scalar
`varip` surviving ticks, per-call-site series for function parameters, lazy `and`/`or`/`?:`. What is
wrong falls in four groups:

1. **One outright logic bug** in the strategy order book: placing an entry deletes pending exits, and
   placing an exit deletes pending entries (finding 1).
2. **The rollback exceptions are mostly missing or partial**: `varip` is per name not per call site,
   cannot be a UDT field, and a `varip` reference dangles; orders do not survive a rolled-back tick;
   runtime errors do not halt the script (findings 2–4, 11–14).
3. **`[]` is not the full time-series model**: it works on a fixed list of built-ins, treats locals and
   `f()[n]` trails differently from TradingView's per-scope buffers, and there is no buffer model at
   all (findings 5–8, 18–21).
4. **Silent differences**: several settings are accepted and dropped without the `ignored_features`
   warning the project requires (finding 15), and no execution-model compiler warnings exist
   (finding 19).

## Findings

Severity: **bug** = the engine computes something TradingView does not and a script can show it;
**diff** = a different but defensible rule or an inferred choice; **gap** = a feature or check absent.
*Fix* names the stacked PR that addresses it (`—` = deferred, see "Not planned yet").

| # | Sev | Area | Finding | Verified | Fix |
|---|-----|------|---------|----------|-----|
| 1 | bug | strategy | `Broker::place` retain predicates are inverted | re-read | PR 2 |
| 2 | bug | varip | `varip` in a function is shared across call sites | audit, run (test) | PR 5 — **done** |
| 3 | gap | varip | `varip` UDT fields do not parse | re-read (parser), run (test) | PR 5 — **done** |
| 4 | bug | varip | `varip` array/map/matrix/UDT handle dangles after a tick | audit, run (test) | PR 5 — **done** (drawings not covered) |
| 5 | bug | history | `[]` on most built-ins is `na` forever | audit, run (test) | PR 4 — **done** |
| 6 | bug | scopes | Block locals keep stale values; one flat history table | audit | — |
| 7 | bug | scopes | Function block locals push `na` on skipped bars | audit | — |
| 8 | bug | scopes | `f()[n]` / `ta.x()[n]` use global offsets; chained `[]` collide; offset 0 skips the trail | audit | — |
| 9 | bug | ta | `ta.atr` takes the previous close from its own call history | audit | — |
| 10 | diff | ta | `ta.rsi` is a simple average, not Wilder's RMA | re-read, needs TV | — |
| 11 | bug | errors | A failed historical run resets state; the script resumes at `bar_index` 0 | audit, run (test) | PR 6 — **done** |
| 12 | bug | errors | A failed closing run does not commit but clears `live_open_time` | audit | PR 6 — **done** (the script no longer runs again) |
| 13 | diff | errors | A realtime runtime error does not halt the script | audit, run (test) | PR 6 — **done** |
| 14 | diff | strategy | The broker is rolled back on unconfirmed ticks | audit | — |
| 15 | gap | strategy | `calc_on_order_fills` and `close_entries_rule = "ANY"` accepted and silently dropped | audit | PR 3 (warning) — **done** |
| 16 | diff | strategy | `process_orders_on_close` fills market orders only | audit | — |
| 17 | diff | declaration | `calc_bars_count` ignored (already warned) | audit | — |
| 18 | gap | buffers | `max_bars_back` is a silent no-op; history is unbounded | audit | none: harmless superset (see PR 3 note) |
| 19 | gap | warnings | No execution-model compiler warnings | audit | — |
| 20 | diff | ta | `ta.*` in a loop pushes one state entry per iteration | audit | — |
| 21 | diff | scopes | A function called twice per bar from one site: the second call reads the first's value | audit | — |
| 22 | diff | security | `request.security` is evaluated only when its site runs | audit | — |
| 23 | diff | ta | `na` inside a window: sma/highest/lowest skip it, wma/stdev/… return `na` | audit, needs TV | — |
| 24 | diff | controller | A bar closes by wall clock; a late final tick forces a full rebuild | audit | — |
| 25 | diff | controller | A new bar first seen already closed runs as `Historical` | audit | — |
| 26 | diff | alerts | `alertcondition` is fixed to `All`; alerts emitted before an error are dropped | audit | — |
| 27 | gap | strategy | `strategy.exit` trailing arguments raise `PINE9005` | audit | — (`trail_offset` alone is meaningless in TradingView too) |
| 28 | gap | type check | A history offset's type is not checked at compile time | audit | — (needs the corpus, see PR 4 note) |
| 29 | gap | strategy | No margin model; `max_*_count` not capped at TradingView's limits | audit | — |
| 30 | diff | controller | Stale `last_valid` output shows after a failed or new run | audit | — (deliberate, see PR 6 note) |
| 31 | gap | perf | Every tick deep-clones `RuntimeState`; every variable's history is pushed every bar | audit | — (see PR 4 note) |

### Strategy

**1. `Broker::place` retain predicates are inverted** (`src/pine/broker.rs:379-388`, bug, re-read).
`retain` keeps what the predicate returns true for. For `Entry | Order` it keeps
`o.kind.is_entry_like() && o.id != order.id`, which drops every pending Exit and Close. For `Exit` it keeps
`o.kind == Exit && o.id != order.id`, which drops every pending Entry. `Close` has the same shape.
`CloseAll` (the line after) is written correctly. TradingView replaces only the pending order of the same
kind and id; an exit placed beside a pending entry waits for it to fill. Then `cull_exits` (`:507`) removes
the orphaned exit too. Existing tests miss it because they guard the exit with `strategy.position_size > 0`
(`pine_host.rs:415`) or place it a bar later (`language_tests.rs:2033`).

```pine
strategy("t")
if bar_index % 10 == 0
    strategy.entry("L", strategy.long)
strategy.exit("X", "L", stop = close - 1, limit = close + 1)
```

The entry never fills. Related and unconfirmed: `place` keys exits by `id` only, so
`strategy.exit("X", "A")` followed by `strategy.exit("X", "B")` replaces the first.

**14. The broker is rolled back on unconfirmed ticks** (`state.rs:117`, `session.rs:643, 676-681`).
`Broker` lives in `RuntimeState`, and every non-historical execution starts from `committed.clone()`.
With `calc_on_every_tick = true` an order placed on tick N is gone on tick N+1 unless the script places it
again; fills made on a tick are replayed from committed state. TradingView: orders placed or filled on
realtime ticks are not rolled back. By default unconfirmed ticks are skipped for strategies
(`execution.rs:335-348`), so this shows only with the option on. Realtime fills use the OHLC heuristic on the
partial candle, so a fill price can change between ticks.

**15. `calc_on_order_fills` is silently ignored** (`compiler.rs:259`; `apply_strategy_setting`,
`compiler.rs:417-418`). The name is accepted and never stored; there is no `ignored_features` entry, which
contradicts "Unsupported behavior is never silent" in `docs/agent-guides/pine.md`. TradingView re-executes the
script on each fill tick (up to four times per historical bar, with rollback between), so an order placed
after the open fill can fill on the same bar's high, low or close. Ours runs `broker.process`, then the script
once, then commits (`session.rs:677-701`). `close_entries_rule = "ANY"` is dropped the same way (it behaves as FIFO).
`fill_orders_on_standard_ohlc` is a documented no-op (`docs/chart-types.md`: strategies fill at a price-based
chart's constructed prices and the report says so), so it stays quiet; the first audit pass listed it in error.

**16. `process_orders_on_close` fills market orders only** (`session.rs:726`, `broker.rs:472`).
`fill_market_orders_at_close` uses `is_market()`, so a limit or stop order the close already satisfies waits
for the next bar.

**27. Trailing stops.** `trail_price` / `trail_points` raise `PINE9005` (`strategy_builtins.rs:563-571`),
which stops the script on that bar. `trail_offset` only applies together with one of them in TradingView, so
its being ignored alone is not a difference.

**29. Margin and counts.** No margin model (default 100 % in TradingView); `equity()` can go negative; an
oversized `default_qty_value` opens a larger position. `max_*_count` is honoured but not clamped to
TradingView's caps (`drawings.rs:387-402`). The stale comment at the top of `strategy_builtins.rs` says
`strategy.risk.*` is accepted and ignored; it is enforced.

### Rollback, `var` and `varip`

**2. `varip` in a function is shared across call sites** (`session.rs:1607-1626, 1656, 2850-2856,
2936-2944`, bug). `Session.intrabar` is a flat name-keyed map. `run_declaration` reads it before the
per-site stash that `invoke_with` restores, so the first call site writes `intrabar["n"]` and the second
adopts it. Tick re-injection (`session.rs:660-664`) also writes the bare local name into the global variable
table.

```pine
f() =>
    varip int n = 0
    n += 1
    n
plot(f()); plot(f() * 10)
```

TradingView: 1, 2, 3… and 10, 20, 30…. Ours: the second call starts from the first call's value.

**3. UDT `varip` fields do not compile** (`parser.rs:1464`, `qualifier_from_name` `parser.rs:1611`, gap,
re-read). `type_field` calls `skip_qualifier`, which only skips `const`/`simple`/`series`/`input`; `varip` is
a separate token and the parser reports `PINE2015`. A `varip` field is TradingView's only way to keep object
state across rollback, so rule "changes to `var` objects revert, except `varip` fields" has no second half.
A fix needs storage outside `instances`, which is rolled back.

**4. A `varip` reference dangles** (`session.rs:660-664, 1836`, bug, audit; TradingView's result for this case
was not confirmed). `intrabar` stores the handle; the array, map, matrix or object lives in `working.*`,
which is restored from `committed`. If it was created on the open bar, the next tick holds a handle to
nothing (array builtins `unwrap_or_default`, so it reads as empty), and `next_reference` is rolled back too,
so a new object can receive the same id and alias the old handle.

```pine
varip array<int> a = array.new<int>()
array.push(a, bar_index)
plot(array.size(a))
```

**13. A realtime runtime error does not halt the script** (`execution.rs:246-250`). A failed tick returns
`Err` and sets `last_error`; the next update re-clones `committed` and runs again, and the error clears if
the condition went away. TradingView stops until reload.

**11. A failed historical run resets state** (`session.rs:687-693`, `execution.rs:277, 293-297`). A failed
replay bar sets `committed = fresh_state()`. `rebuild` still stores `self.bars`, so the next appended closed
bar passes `can_apply_incrementally` and executes as `Historical` with `bar_index == 0` on empty state. The
output after the failure is therefore empty (the doc comment on `run_script_checked`, `host/pine_host.rs:238`,
says "whatever the script had drawn up to that bar"). The test `a_failure_clears_once_the_script_runs_clean`
encodes the resumption. TradingView halts and keeps the plots before the failing bar.

```pine
if bar_index == 5
    runtime.error("x")
plot(close)
```

**12. A failed closing run** (`execution.rs:207-209, 224`) clears `live_open_time` but does not advance
`committed.bar_index`, so the next bar runs with the failed bar's index and the failed bar has no history.
Only on the incremental path (`next[len-2] == old_last`).

**30. Stale output** (`chart_card.rs:1216-1222`): `last_valid` is kept across restarts, so the old source's
output stays on screen while a new run is pending and stays if the new run fails.

### Time series and scopes

**5. `[]` on most built-ins is `na` forever** (`session.rs:1246-1273`, `commit_histories` `3024-3094`, bug).
`eval_history` keys an `Identifier` base by name; `commit_histories` records only
`open/high/low/close/volume/hl2/hlc3/ohlc4/time/time_close/bar_index`, five `strategy.*` names
(`STRATEGY_SERIES`, `session.rs:40-46`) and `working.variables`. Built-ins are not stored in `variables`
(`market()`, `session.rs:1047+`). So `hlcc4[1]`, `dayofweek[1]`, `hour[1]`, `ta.tr[1]`,
`barstate.isfirst[1]`, `timeframe.period[10]`, `syminfo.*[1]`, `last_bar_index[1]`, `ta.obv[1]` and
`strategy.closedtrades[1]` / `grossprofit[1]` / `initial_capital[1]` are `na` on every bar. TradingView gives
every non-void expression a history trail.

```pine
plot(hlcc4[1] == hlcc4[1] ? 1 : 0)   // 0 (na == na is false)
```

**6. Block locals keep stale values** (`session.rs:1480-1520`). Locals are not removed after their block;
`shadowed_by` saves only names that already exist, and the flat `variables` map keeps the stale value, which
`commit_histories` pushes every bar. `v[1]` happens to match TradingView; `v[2]` and further differ. Two blocks
that declare the same name share one history.

```pine
if bar_index % 3 == 0
    v = close
    plot(v[2])      // TV: close from 6 bars ago. Ours: from 3 bars ago.
```

**7. Function block locals push `na` on skipped bars** (`session.rs:2848, 2899-2921`). Locals are removed at
call start, so a local from a skipped `if` reads `Na`, and `invoke_with` pushes that into its history.
TradingView keeps the last committed value, so `t[1]` is the close of the previous bar the block ran on; ours
is `na`.

**8. Expression trails use global offsets** (`session.rs:1241-1266, 3078-3092`, `parser.rs:1003-1007`).
`commit_histories` pushes `na` for every `expr:` key on bars where that site was not evaluated, so
`f()[1]` / `ta.sma(...)[1]` in a conditional position differ from TradingView's per-scope buffer. `x[0]`
returns before recording the base, so a site read with offset 0 early and 1 later has an incomplete trail
(`(close*2)[bar_index < 5 ? 0 : 1]` gives `na` at bar 5). In `f()[1][2]` both nodes share a start offset and
therefore an `expr:` key, so the trails overwrite each other.

**20, 21.** A `ta.*` call inside a loop pushes one entry per iteration into the call site's state
(`ta.rs:101-104`), so `for i = 0 to 4: ta.sma(close, 3)` averages repeated entries of one bar. A user function
called twice per bar from one site: the second call reads the first call's pushed value
(`session.rs:2905-2918`). TradingView warns about the loop case; the exact rule for the second is unknown.

**19. No execution-model warnings.** TradingView warns when a history-reading call (`ta.*`, `[]` on a local)
sits in a local scope or an operand of `and`/`or`/`?:`. The only warning producers are
`ignored_features.rs` (ignored arguments) and `structure_validator.rs` (`PINE7006`). `x = close > open ?
ta.sma(close, 5) : na` and the loop above compile silently.

**18. Buffers.** History is unbounded: no 5000-bar cap, no sizing from the first 244 bars, no `RE10143`, no
memory error. The `max_bars_back(series, n)` builtin is a no-op (`misc_builtins.rs:80-83`) and the
declaration argument is stored (`program.rs:105`, `compiler.rs:358`) and never read, with no warning
(`ignored_features.rs:88-110`). This is a superset of TradingView's behaviour: no script that runs on
TradingView can read less history here, so a warning would only be noise. Only a script that TradingView
stops with a buffer error runs here. Not fixing.

**28. Offset type.** `type_checker.rs:583-589` infers the offset but does not require an int. `close[1.5]`
returns `na` at runtime (`session.rs:1224-1231`); TradingView rejects it at compile time. `close[-1]` is a
runtime error (`PINE4006`), a `na` offset returns `na`.

**22. `request.security`** (`security_call.rs:482-545`). `serve_security` folds the higher-timeframe candle
from only the chart bars visited at the call site, so a conditional call excludes the skipped bars from the
candle's high, low and volume and from any `ta.*` inside the expression. TradingView evaluates the
expression on every higher-timeframe bar and the condition only selects the result. Chart variables used
inside the expression have no history (`security_globals` vs `eval_history`).

**31. Cost.** `commit_histories` pushes every variable every bar even if never indexed (TradingView buffers
only what is referenced). Each non-historical tick does `working = committed.clone()` (`session.rs:643`) and
the close clones again (`:704`), O(history × variables) per tick. `ta.macd` (`ta.rs:836`) and the
`ta.obv`-family `cumulative` (`ta_extra.rs:650-700`) re-read the whole history each bar.

### Indicators

**9. `ta.atr`** (`ta.rs:549-551`) computes the true range with `state.closes.last()`, the close at the
previous *call*, not the previous chart bar. Called conditionally (`if bar_index % 2 == 0`), gaps are ignored.
`ta.tr` reads the real previous close (`market_values.rs:137-141`).

**10. `ta.rsi`** (`ta.rs:777`) is the simple average of gains and losses over `length + 1` non-`na` values.
The module header and the test `rsi_is_the_simple_average_variant_and_saturates` say this is a deliberate
"faithful quirk" of the Swift app. TradingView uses `rma(up) / rma(down)`, so values differ from the first
bar. Outside the execution model; listed because it is user-visible.

**23. `na` in a window.** `sma`, `highest`, `lowest` and `rsi` use `last_valid` (`ta.rs:648-684, 707`), which
compresses over `na`; `wma`, `stdev`, `median`, `variance`, `cci` use the strict `last_window`. The module
header says "a window with any `na` yields `na`", so the file contradicts itself. A test pins the
`last_valid` behaviour (`a_gap_in_the_source_delays_a_window_that_skips_na`). TradingView's rule should be
checked (believed: `ta.sma` is `na` if any element is `na`).

### Controller, alerts, declaration

**24, 25.** `is_closed` is `bar.close_time <= now_ms` (`execution.rs:357`). If the feed's final tick arrives
after `close_time`, the bar commits stale; the corrected bar then has the same `open_time` and no
`live_open_time`, so `can_apply_incrementally` is false (`execution.rs:288-291`) and the controller does a
full rebuild, which restarts logs (`session.rs:431`), `var`, `varip` and replays previously realtime bars as
historical. A new bar first seen already closed runs as `Historical` (`execution.rs:227-233`): `isrealtime`
false and `alert()` never emits (`alert_builtins.rs:112`); a missed intermediate bar fails
`next.len() == self.bars.len() + 1` and also rebuilds.

**26. Alerts.** `alertcondition` is hard-wired to `AlertFrequency::All`. `emitted_alerts` is forwarded only on
`Ok` (`execution.rs:311`) while the `once_per_bar` ledger is consumed (`alert_builtins.rs:94`), so an alert
fired just before a runtime error is lost for that bar. A rebuild suppresses alerts for the whole update with
the ledger already consumed.

**17. `calc_bars_count`** is ignored with a `PINE7001` warning (`ignored_features.rs:99`); the script always
runs over the whole history, so `barstate.isfirst` and `var` initialisation are at the chart's first bar.

**16** and **15** are described under Strategy.

## Checked and matching TradingView

- One execution per historical bar; built-ins refreshed first; non-`var` variables re-declared; `var`
  guarded by `working.variables.get(name)` (`session.rs:1602`); declaration calls evaluated at compile time.
- `varip int n = -1; n += 1` equals `bar_index` on historical bars (`intrabar` cleared per bar,
  `session.rs:624`).
- Commit only on a confirmed bar; every unconfirmed tick and the closing run start from `committed`;
  `ta.*` state does not leak across ticks (`session.rs:3444`); no `static` / `thread_local` state under
  `src/pine`.
- Rolled back: variables, histories, `ta_states`, plots, `bgcolor`/`barcolor`/`fill`, arrays, maps, matrices,
  UDT instances, all drawings (and their per-type counts), `securities`, visual alert list, broker,
  `once_finished`, `next_reference`. Outside: `Session.logs`, `footprints`, `once_per_bar_ledger`,
  `emitted_alerts`, `intrabar`, `last_bar_index`/`last_bar_time`, pure caches.
- Scalar `varip` survives ticks (`session.rs:660-664`; `debug/realtime_tests.rs:142`).
- `log.*()` persists across rollback; alert frequencies (`once_per_bar`, `once_per_bar_close`, `all`),
  historical bars never emit.
- `barstate.*`: first tick `isnew` true / `isconfirmed` false / `isrealtime` true / `ishistory` false; later
  ticks `isnew` false; closing run `isconfirmed` true; last historical bar `islastconfirmedhistory`;
  `isnew` is derived from `open_time` (`session.rs:622`).
- Input change → new epoch, rebuild, last open bar live; duplicate or stale bars rejected
  (`session.rs:608-621`); a timeframe change swaps candles and rebuilds; `visible_range` rebuilds only when
  the script reads it.
- Function and method parameters get a per-call-site series written only on bars where the call runs,
  even for global arguments (`history_locals`, `session.rs:2864-2921`; TradingView's `upDownColor` example).
- `ta.*` state is per call site (`site_key(site, context)`) and advances only when the call runs, so a
  conditional `ta.sma` uses a buffer of the called bars only. `ta.change`, `crossover`, `barssince`, `fixnan`,
  `math.sum` are stateful per site; other `math.*` are stateless (`call_binding.rs:97-107`). Cumulative
  variables catch up on unread bars (`ta_extra.rs:640-700`). `ta.ema` / `ta.rma` seeding.
- `and` / `or` / `?:` are lazy (`session.rs:1195-1207, 855-868`).
- Global `x[n]` is a plain bar offset, `na` before the first bar; `(open[10])[1]` and
  `ta.highest(high, 20)[10]` work at global scope.
- Broker: O-H-L-C path when `|O-H| < |O-L|`, else O-L-H-C (`broker.rs:463`); gap fills at the open
  (`reach`, `broker.rs:969`); a market order fills at the next open; an opposite `strategy.entry` reverses
  in one step; exits can fill on the entry's bar; `strategy.risk.*` enforced.

## Not planned yet

Deferred, each because it changes the history model, needs a TradingView comparison, or is large:

- Local-scope buffers (6, 7, 8, 21): a change of `commit_histories` and `eval_history`; bench-sensitive.
- A broker that survives rolled-back ticks and real `calc_on_order_fills` re-execution (14, 15).
- `process_orders_on_close` for conditional orders (16); trailing stops (27, beyond the warning).
- `ta.rsi` → RMA (10), the `na`-window rule (23), `ta.atr` previous close (9): compare with TradingView
  output first. Changing `ta.rsi` changes plotted values and strategy results of saved scripts.
- Execution-model compiler warnings (19, 20); a buffer model with `max_bars_back` (18, beyond the warning).
- `request.security` conditional folding (22); `alertcondition` frequency and pre-error alerts (26);
  closing by feed flag, already-closed new bars (24, 25); margin model and count caps (29); the per-tick
  state clone (31, first half).

## Fix plan

Stacked pull requests, each from the previous one. Every PR runs `tools/pine-bench.sh <base> <head>` and
pastes the table; existing workloads must stay within ±3 % (the measured noise is ±2.5 %), checksums must not
change except on the workload the PR targets, and PRs touching state or rollback also run with
`PINE_BENCH_FEATURES=alloc-count`.

1. This document and the new bench workloads (`strategy_entry_exit_same_bar`, `varip_sites`,
   `builtin_history_reads`, `index_only_vars`, `error_halt`, `realtime_strategy_ticks`).
2. Broker order book (1).
3. Warnings for silently ignored settings that change results (15). Done: `calc_on_order_fills = true` and
   `close_entries_rule` other than `"FIFO"`. Not warned, on purpose: `max_bars_back` (18), lone `trail_offset` (27),
   `fill_orders_on_standard_ohlc` (documented no-op).
4. History of built-in variables (5). Done: the program is scanned once for `name[n]` where `name` is a builtin
   variable (`symbol_catalog::is_variable`) that `commit_histories` does not record itself; those names are
   read at the end of every confirmed bar (`Session::read_history_builtins`) and kept like variables. A script
   that indexes none pays one `is_empty()` test per bar. Not done, on purpose: the compile-time int check on
   the offset (28) could reject a script TradingView accepts if the type checker infers `float` where
   TradingView has `int` (e.g. `math.round`), and the corpus that would show it is not cached here; and
   recording history only for indexed variables (31) would break debugger watch expressions
   (`pine::debug::parse_expression` can index any variable) unless gated on the hooks. Limitation: inside a
   `request.security` expression, a builtin's history is still `na` (the site calls `commit_histories`
   without builtins).
5. `varip` per call site, UDT `varip` fields, reference-typed `varip` (2, 3, 4). Done. A `varip` local of a
   function is kept under its call site's key (the one `invoke_with` stashes persistent locals under), so
   two calls and a same-named global no longer share it, and it survives the ticks of a bar. `varip` is
   accepted on a UDT field (`TypeField::persistent`). `Session::capture_intrabar_objects` keeps, after each
   unconfirmed tick, what every `varip` variable points at (array, map, matrix, object) and every `varip`
   field of every object; `restore_intrabar_objects` puts them back after the next tick's rollback, bumping
   `next_reference` past them. What the container holds is followed too, through arrays, maps, matrices and
   objects (a visited set stops a cycle), so an array in a `varip` array, behind a `varip` field, or made on
   a rolled-back tick comes back with its contents and a live handle. The `varip` fields of types an imported
   library exports are found through the linked type table. Fields go back only into an object the committed state has (an object made on
   the rolled-back tick is gone, as in TradingView, and its handle may belong to another object by now).
   Inferred, not confirmed against TradingView: a `varip` array keeps its contents, not only its handle.
   Not covered: a `varip` variable holding a drawing (lines, labels, boxes, tables and the like are not
   followed and roll back with the tick).
6. A runtime error halts the script (11, 12, 13). Done: `Session::halted` is set when a run fails (not when it is
   cancelled); `execute` then returns the stored error without running, `reset` (every rebuild, input change,
   symbol or timeframe change, empty snapshot) clears it. `output()` shows the bars before the failing one
   (`bar_count`, plots truncated), where a failed replay used to leave an empty chart. A failure on a live tick
   keeps `committed` as the state, so nothing half-run is shown. The same holds for a closed bar that fails
   when it arrives on its own: `execute` keeps a copy of the state it continued from and `halt` puts it back.
   A failed history replay has no such copy (one per bar would slow every run), so `load_history` runs the bars
   before the failing one again (`settle_failed_replay`) with the debugger, profiler and logs set aside: every
   category of output (labels, lines, boxes, tables, markers, backgrounds, alerts, the strategy report) is then
   what the last good bar left. This costs a failing script a second pass over its history up to the failure,
   and nothing on the way to success. Behaviour change worth knowing: a run that
   failed on a transient condition (for example `request.security` to a finer timeframe before its candles
   have loaded, `PINE4021`) used to be retried by the next appended bar; it now stays stopped until the chart
   rebuilds, as in TradingView. Not changed: `ChartCard` still shows the last valid output of the same chart
   while a failed run is current (`chart_card.rs`, finding 30): it is what keeps a chart from blanking while
   a script is being edited, and it cannot be checked without the app.

## Not verified

- Nothing here was executed. Repro snippets are predictions from reading the code paths.
- No finding was compared against TradingView output. Items marked *needs TV* and the reference-typed
  `varip` result in particular.
- The Pine Script documentation page does not specify the buffer details for built-in names or the `ta.sma`
  `na` rule, so findings 5–10 and 23 depend on TradingView behaviour outside that page.
