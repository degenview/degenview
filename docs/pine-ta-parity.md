# `ta.*` against TradingView: what is checked and what still differs

Two groups are covered: the true-range family and the functions that meet a missing (`na`) source
bar ([below](#a-missing-source-bar)).

The true-range family (`ta.tr`, `ta.atr`, `ta.kc`, `ta.kcw`, `ta.dmi`, `ta.supertrend`) was brought to
TradingView's output in the PR that added this file. This page records what was compared, how, the
differences that are left, and how to measure more. Read it before changing one of these built-ins or
before fixing a difference below.

## What the engine does

- One definition of the true range: `ta::true_range_checked(bar, previous_close, handle_na)`
  (`src/pine/builtins/ta/mod.rs`). `na` on a bar with no previous close unless `handle_na`.
  `ta.tr`, `ta.tr(handle_na)`, `ta.kc`, `ta.kcw` and `ta.dmi` seed from the `na` form; `ta.atr` and
  `ta.supertrend` use `high - low` on bar 0 (TradingView's reference code calls `ta.tr(true)`).
- The previous close is the chart's `close[1]` (`market_values::previous_close`), never a per-call-site
  copy, so a call under an `if` measures against the right bar.
- `ta.dmi` keeps its four smoothed series in its own `State::dmi`. It still takes the previous high
  and low from the previous *call*: TradingView does the same (a conditional `ta.dmi` matches, see below).
- The Keltner basis is the moving average on its own (first value one bar before the bands).

## Compared with TradingView (BINANCE:BTCUSDT 5m, bars from 2026-11-02 00:00)

Every comparison uses TradingView's own Pine logs of the same bars; the log is the fixture, so the
test needs no network.

| Test | Fixture | Covers |
| --- | --- | --- |
| `the_true_range_family_matches_tradingview_on_its_first_bars` | inline | bars 0-8, short lengths: `tr`, `tr(true/false)`, `atr(3)`, `supertrend(2, 3)`, `kc(3)`, `kcw`, `dmi(3, 3)` |
| `the_true_range_family_matches_tradingview_over_eighty_bars` | `src/pine/tests/data/tradingview-true-range-family.log` | default lengths over 80 bars, and the same calls under `if bar_index % 3 == 1` (conditional `atr`, `kc`, `dmi` match; conditional `supertrend` is skipped, see below) |
| `the_true_range_family_matches_tradingview_with_other_inputs_and_deeper_history` | `src/pine/tests/data/tradingview-true-range-inputs.log` | 200 bars: `kc(.., false)` and its `kcw`, `dmi(7, 20)`, the defaults out to bar 199 |

Also checked, against the Pine reference code written in the same script rather than TradingView:
a live bar at its four stages (`the_true_range_family_follows_a_live_bar_like_its_reference_code`),
replay-style shorter charts, `request.security`
(`the_true_range_family_inside_a_request_measures_the_requested_bars`), and flat bars and gaps
(`the_true_range_family_follows_its_reference_code_over_flat_bars_and_gaps`).

## Differences that are left

1. **A conditional `ta.supertrend` does not match TradingView.** In the 80-bar fixture the `B` lines'
   `gSt` column is TradingView's `ta.supertrend(3, 10)` called on bars where `bar_index % 3 == 1`.
   TradingView prints `na` on calls 1-3 (bars 1, 4, 7), `0` on bars 10, 16 and 22, `78123.805` on bars 28
   and 31, then a band that moves (`78134.7589`, `78178.29251`, `78277.241259`, `78354.6386331` held for
   bars 43-61, ...). That is neither what the reference code gives when run conditionally (what the
   engine does, pinned by `a_conditional_supertrend_follows_the_reference_code`) nor the chart's band.
   `ta.supertrend` unconditionally matches on all 200
   bars. The test skips the column (`the_true_range_family_matches_tradingview_over_eighty_bars`).
   To fix it: probe a conditional call with a gap in the condition (every call, every 2nd, every 3rd)
   and log `ta.atr` inside the same `if`, to see which of TradingView's internal series the built-in
   reads per chart bar and which per call. Remove the skip once it matches.
2. **`ta.sar` keeps its own `highs`, `lows` and `closes` per call site** (`evaluate_sar`), the model
   that `atr`, `kc` and `dmi` had. Not compared with TradingView, conditionally or otherwise. Probe it
   first: if TradingView's conditional `ta.sar` differs from an unconditional one, move it to the chart's
   previous bars the same way.
3. **Flat bars and gaps were not in the probe chart** (no `high == low` bar, no open far from the last
   close in 200 bars of BTCUSDT 5m), so those are checked against reference code only. A thin market
   (a halted stock, a small pair) or a stock chart across the overnight gap would show them.
4. **Higher-timeframe bars in the unit-test harness.** In a 1-minute chart with a `"5"` request, the
   harness's walk provider showed each requested close from the minute after the previous one, not the
   minute the requested bar closes. Not investigated; the harness provider is synthetic. Check with real
   data and TradingView before treating it as an engine bug (see
   `the_true_range_family_inside_a_request_measures_the_requested_bars`, which avoids the question by
   comparing with reference code inside the same request).

## Loose ends in the code (not bugs)

- `market_values::previous_close(&self.working)` is read at nine call sites and passed down as a
  parameter (`evaluate_price`, `evaluate_supertrend`, `evaluate_atr`, `evaluate_dmi`, `keltner`). A single
  per-bar context carrying it would be tidier.
- `true_range` (falls back to `high - low`) and `true_range_checked(.., handle_na = true)` say the same
  thing; only `ta.atr` uses the first.

## How to measure TradingView again

The built-in browser is signed in to TradingView (free plan: no custom intervals).

1. Write a probe that `log.info`s one line per bar (`"A" + bar_index + " " + open + ...` for the data,
   `"C" + bar_index + " " + values` for results). Use `str.tostring(x, "#.########")` so `na` prints `NaN`.
2. Paste it into the Pine editor with a `ClipboardEvent('paste')` after a real click and `cmd+a`, click
   "Update on chart" twice, then open the editor's "..." menu, "Pine logs".
3. The logs list is virtualised: scroll its container in steps of about 250 px and collect the lines from
   the DOM into a `Map` keyed by `A<n>` / `C<n>` until none is missing. A fetch or image beacon to
   `localhost` is blocked by the page's content-security policy, so the data has to come out through the
   tool result (check it with a rolling checksum on both sides).
4. Save the bars and results as a fixture under `src/pine/tests/data/`, load it with `include_str!`, run
   the script over the bars with `run_script`, and compare with the print precision (eight decimals).

## A missing source bar

`ta.ema`, `ta.rma`, `ta.macd`, the averages inside `ta.kc`, `ta.tsi` and `ta.rsi`, and the window
statistics follow TradingView's rules for a `na` source bar (rules in
[Pine and scripts](agent-guides/pine.md)). Compared with TradingView on BINANCE:BTCUSDT 5m, the first
120 bars from 2026-11-02 00:00, with `src = bar_index % 37 == 0 ? na : close` (holes at bars 0, 37, 74
and 111): `src/pine/tests/data/tradingview-missing-bars.log`, test
`a_missing_source_bar_gives_what_tradingview_gives`. 28 of the 42 logged columns match on every bar:
`ta.ema(10)`, `ta.rma(10)`, `ta.rsi(14)`, `ta.stdev`, `ta.variance`, `ta.median`, `ta.range`,
`math.sum`, `ta.wma`, `ta.dev` (strict), `ta.bb` upper, `ta.bbw`, `ta.macd` (line, signal,
histogram), `ta.kc` basis and upper, `ta.tsi`, `ta.cci`, `ta.cog`, `ta.wpr`, `ta.linreg`,
`ta.mom`, `ta.roc`, `ta.change`, `ta.sma`, `ta.swma`, `ta.alma`.

Also checked against Pine reference code rather than TradingView: a run of four holes, a live bar,
prefix stability (a bar shows the same value on a shorter chart), `request.security` over a source
with holes, odd lengths (1, longer than the chart) and a source that never has a value, and a real
window on live BTC weekly bars (`ta.ema`, `ta.rma`, `ta.stdev` equal their definitions over 264 bars with
8 holes).

### Differences that are left

The other 14 columns differ; the test skips them (`NOT_YET_LIKE_TRADINGVIEW`). What TradingView
printed, with the engine strict (`na` until `length` bars without a hole) for all of them:

| Function | TradingView over a hole | Engine |
| --- | --- | --- |
| `ta.cmo`, `ta.correlation`, `ta.hma`, `ta.mode`, `ta.vwma`, `ta.mfi` | a value from the first bar after the hole (`ta.mfi` also from bar 14, one bar earlier) | `na` for `length` bars after it |
| `ta.percentrank` | a value on and after the hole, and from bar 10 | `na` there |
| `ta.highest`, `ta.lowest` | skip the hole; a value from bar `length - 1`, though the bar 0 value is `na` | needs `length` values that exist |
| `ta.highestbars`, `ta.lowestbars` | same, with the offset counted in chart bars | same as above |
| `ta.rising`, `ta.falling` | skip the hole: the bar is greater than (less than) the greatest (least) value that exists in the window | `false` while a hole is in the window |
| `ta.cum` | `na` on the hole bar, the running total carries on after it | repeats the total on the hole |

To fix one: change its rule where its window is built (`last_window` against `last_valid` in
`src/pine/builtins/ta/mod.rs`), move its column out of `NOT_YET_LIKE_TRADINGVIEW`, and update this
table.

### Calls made on some bars only

Test `conditional_calls_over_a_missing_bar_match_tradingview`, log
`src/pine/tests/data/tradingview-missing-bars-conditional.log`: the functions called inside
`if bar_index % 3 == 1` over `src = bar_index % 10 == 4 ? na : close` (holes on bars 4, 14, ..., so some
fall on a call bar and some between two). `ta.ema`, `ta.rma`, `ta.rsi`, `ta.stdev`, `ta.variance`,
`ta.median`, `ta.range`, `math.sum`, `ta.bb`, `ta.macd` and `ta.tsi` match TradingView on every bar
they ran on: they read the series of their own calls, as the engine does.

**`ta.wma` does not.** On the same calls TradingView prints `na` on the first five and then values that
no simple rule gave: not the last five valid calls (weights 1 to 5), not the last five chart bars with
the hole filled or skipped, not the unconditional fill rule, and no combination of five valid closes
fits the numbers (for example bar 19: TradingView `78567.34`, last five valid calls `78444.917`,
chart bars `78537.52`). The test skips that column. To fix it, probe `ta.wma` conditionally without a
hole first, to see how its window reads a conditional series, then with the hole.
