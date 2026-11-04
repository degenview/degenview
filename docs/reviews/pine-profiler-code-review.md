# Pine profiler production-readiness review

Date: 2026-11-04  
Reviewed revision: `d782f68` (`main`)  
Scope: Pine profiler region discovery, runtime hooks, recorder/accounting, threaded sessions,
`request.security()` wait measurement, Pine Studio lifecycle and staleness, editor marks, Profile
drawer UI, AI assistant integration, documentation, and focused tests.  
Review type: static review plus focused automated checks; no implementation changes, corpus run,
benchmark rerun, live-provider fault injection, or manual GUI pass.

## Verdict

The profiler has a strong engine core, but it is **not production-ready yet**. Its opt-in hook design
is disciplined, the recorder has bounded memory, statement/function nesting is modelled cleanly, and
the stepping-clock tests enforce the normal-run accounting identity exactly. The equivalence tests
also cover useful indicator, strategy, container, history, function, and `request.security()`
workloads.

The release-blocking issue is not the timing stack; it is whether the result describes the run that
the UI says it does. A profile can remain `Ready` after OHLCV values change, after the last candle
changes from realtime to closed, after the theme changes, or after the underlying market source
changes. In addition, scripts that read the visible chart range are profiled with no visible range,
so they can execute different branches from the preview being diagnosed. A profiler whose central
promise is “this is where the current preview spends its time” cannot silently use a materially
different execution context.

Partial and exceptional runs need another hardening pass too. A cancellation or timeout detected
before a bar starts still increments the bar/progress totals, and a panic while a region is open can
produce region statistics greater than the report total while a clamp hides the broken summary
invariant. The overhead estimate also calibrates only the statement path but applies that estimate to
function calls with extra hash-map bookkeeping, affecting both the displayed estimate and deadline
compensation.

Severity meanings used below:

- **High:** the profiler can present materially incorrect measurements as current or profile a
  different execution context from the preview.
- **Medium:** plausible user-visible accounting/diagnostic errors, unreliable exceptional-run
  reports, or measurement bias in a core metric.
- **Low:** persistent avoidable work, presentation inconsistency, documentation drift, or missing
  hardening unlikely to invalidate the common case by itself.

## Findings summary

| ID | Severity | Finding | Status |
| --- | --- | --- | --- |
| H1 | High | The profile key omits candle values, realtime/closed state, theme, and full symbol identity | **Fixed** ([#321](https://github.com/degenview/degenview/pull/321)) |
| H2 | High | Visible-range scripts are profiled with a different context from the preview | **Fixed** ([#321](https://github.com/degenview/degenview/pull/321)) |
| M1 | Medium | Cancellation and timeout can count a bar that never started executing | **Fixed** ([#320](https://github.com/degenview/degenview/pull/320)) |
| M2 | Medium | A panic can produce internally inconsistent region and total times | **Fixed** ([#320](https://github.com/degenview/degenview/pull/320)) |
| M3 | Medium | Overhead calibration does not model function-call hooks and biases attribution/deadlines | **Fixed** ([#320](https://github.com/degenview/degenview/pull/320)) |
| M4 | Medium | Profile failure state uses incompatible notions of “current” in the drawer and Problems list | **Fixed** ([#321](https://github.com/degenview/degenview/pull/321)) |
| L1 | Low | Keeping the profiler enabled allocates and hashes the full identity every frame | **Fixed** ([#321](https://github.com/degenview/degenview/pull/321)) |
| L2 | Low | Stale same-text results are navigable but are not actually dimmed as documented | **Fixed** ([#322](https://github.com/degenview/degenview/pull/322)) |
| L3 | Low | Boundary formatting reports `1000 µs` and rounds sub-100% shares to `100%` | **Fixed** ([#322](https://github.com/degenview/degenview/pull/322)) |

All nine findings were re-checked against the code at `d782f68` before any change; every one is real.
The fixes are three stacked pull requests: accounting (M1–M3, with this file, #320), run identity
(H1, H2, L1, M4, #321) and presentation (L2, L3, documentation drift, #322). The sections below record what was
done and where it differs from the recommendation. Nothing here was exercised in a real window; the
checks are unit tests, the focused suites named under each status, `cargo clippy --all-targets
--all-features -- -D warnings` and `cargo fmt`.

## High findings

### H1 — The profile key does not identify the execution it measured

`ProfileKey` claims that equal keys mean rerunning would “measure the same thing,” but its bar
identity is only `(count, first open time, last open time)`
([profiler.rs](../../src/pine_studio/profiler.rs#L24)). `PreviewState::bar_key` supplies exactly that
tuple ([preview.rs](../../src/pine_studio/preview.rs#L332)). It does not change when any OHLCV value
is corrected or updated, when a forming candle closes, or when an existing candle's close time is
changed without changing its open time.

The key also stores a display string such as `BTCUSDT · 1H`, rather than the full `SymbolInfo`, and
does not include the chart theme. Those values do reach the actual profile run through `DebugFeed`
and `ProfileConfig` ([preview.rs](../../src/pine_studio/preview.rs#L362),
[profiler_host.rs](../../src/pine_studio/profiler_host.rs#L114)), so changing them can change
`syminfo.*`, `chart.bg_color`, `chart.fg_color`, control flow, execution counts, and hot lines without
making the completed profile stale. Changing between two data sources with the same displayed
ticker/timeframe has the same identity problem.

The normal chart execution already has the stronger model the profiler needs: `run_revision` hashes
every candle field and includes visible range plus whether the last bar is closed
([chart_card.rs](../../src/ui/chart_card.rs#L64)); a run is separately keyed by full symbol,
bar-seconds, and theme ([chart_card.rs](../../src/ui/chart_card.rs#L1168)). The profiler duplicates a
weaker identity instead of reusing that source of truth.

Impact: a live price/volume update, corrected historical bar, bar close, appearance switch, or
provider switch can leave the toolbar and drawer saying `Profile ready` while the displayed counts
and rankings describe a different execution. Scripts commonly branch on prices and bar state, so
this is not limited to slightly different nanosecond totals.

Recommendation: expose one immutable preview execution revision and use it for normal execution,
debugging, profiling, and AI profiling. It should cover candle contents, live/closed phase, full
`SymbolInfo`, chart kind/source, effective bar seconds, theme, inputs, and any other Pine-visible
context. Store cheap revision counters or cached hashes in the owners of those values instead of
reconstructing the identity in the profiler. Add tests that mutate OHLCV with unchanged timestamps,
cross the last bar's close time, change appearance, and switch same-ticker sources; each must make
the old profile stale.

### H2 — Visible-range scripts are not profiled as the preview runs them

The normal chart derives the currently visible candle range and passes it to the Pine worker when a
script reads `chart.left_visible_bar_time` or `chart.right_visible_bar_time`
([chart_card.rs](../../src/ui/chart_card.rs#L1138),
[chart_card.rs](../../src/ui/chart_card.rs#L1243)). `ProfileConfig` supports the same
`visible_range`, and the profile session passes it into both compilation/controller construction and
execution ([session.rs](../../src/pine/profile/session.rs#L215)).

The Pine Studio host never populates it. `DebugFeed` and `RunContext` carry no visible range
([preview.rs](../../src/pine_studio/preview.rs#L140)), and `profiler_start` sets inputs, theme,
libraries, and security data but leaves `config.visible_range` as `None`
([profiler_host.rs](../../src/pine_studio/profiler_host.rs#L114)). AI profiling uses the same
incomplete feed.

When the range is `None`, the runtime falls back to the first and last loaded bars. A script whose
visible-range condition selects an expensive calculation can therefore run that calculation in the
preview and skip it in the profiler, or the reverse. The resulting “hottest lines” are about a
different program path.

Recommendation: make the frozen profiling/debug feed the authoritative snapshot of every
Pine-visible chart value, including visible range and effective bar seconds. Populate it from the
same `ChartCard` window used by normal execution, and include it in the revision from H1. Add an
integration test that pans/zooms a script branching on both visible-range builtins and proves the
preview and profiled execution counts match.

## Medium findings

### M1 — An interruption before execution still increments bar and progress totals

`Session::execute` opens the profiler bar before `execute_bar` performs its first cancellation and
deadline check. It then passes `counted = true` for every script diagnostic; only an execution
rejection is uncounted ([session.rs](../../src/pine/session.rs#L565)). Consequently, a pre-raised
cancellation flag or a zero/expired deadline fails at the first line of `execute_bar` but still calls
`ProfileRuntime::end_bar(true, ...)`. That method increments `bars`, `realtime_bars`, `total_ns`, and
the progress atomic even when no statement ran
([recorder.rs](../../src/pine/profile/recorder.rs#L142)).

The same ambiguity affects mid-bar interruptions: partial work is useful and should be retained, but
the UI calls the counter “bars finished” and says “cancelled after N bars.” A bar rejected before any
script work is neither finished nor profiled. Runtime failures have similar wording drift: the tests
explicitly count the failing seventh bar after six successful ones
([tests.rs](../../src/pine/profile/tests.rs#L693)), while the documentation and comments claim the
failing bar is left out.

Impact: partial-profile progress and caveats can be off by one, including a one-bar timed-out profile
with zero executed statements. Failure messages say “after N bars” when the error occurred on bar N.

Recommendation: give bar completion an explicit outcome such as `NotStarted`, `Partial`,
`Completed`, `Failed`, and define separate `attempted_bars`/`completed_bars` semantics if both are
useful. Perform the initial interruption/rejection checks before opening a measured bar, or return
whether execution actually began. Use “failed on bar N after M completed bars” for failures. Add
deterministic tests for a pre-raised cancellation flag, a zero deadline, interruption between bars,
and interruption inside a long bar.

### M2 — Panic recovery can return a report that violates the accounting invariant

The session catches panics around `controller.update` and then still calls `finish_profile`
([session.rs](../../src/pine/profile/session.rs#L248)). If the panic unwinds past `Session::execute`,
`hook_end_bar` is never called. `ProfileRuntime::finish` closes remaining frames and adds their
inclusive/exclusive statistics, but it never adds the open bar to `total_ns`
([recorder.rs](../../src/pine/profile/recorder.rs#L300)). It then masks only the summary by storing
`min(attributed, total)` ([recorder.rs](../../src/pine/profile/recorder.rs#L332)); the per-region
statistics can still exceed the total and the advertised identity
`sum(line time) + unattributed = total` no longer holds.

The Pine Studio currently discards failed reports, which limits immediate UI exposure, but the
engine publicly returns the inconsistent report and the AI/session layers depend on that contract.
Provider mutex poisoning, an assertion, or another runtime bug is exactly when diagnostic data must
remain trustworthy rather than silently clamped.

Recommendation: use an RAII bar guard or catch at a boundary that always finalizes the recorder.
Choose one defined policy for a panicking bar: atomically discard all of its counter deltas, or count
its elapsed time and label it partial. Remove the `min` as a silent repair; assert the invariant in
debug builds and return a clearly invalid/failed outcome if it cannot be satisfied. Add a panicking
clock/provider/runtime test with an open nested region.

### M3 — The overhead estimate and deadline compensation do not model all measured executions

`calibrate_pair_cost` measures only `enter_statement(0)`/`exit()` on one pre-existing region
([recorder.rs](../../src/pine/profile/recorder.rs#L352)). The resulting single number is multiplied
by every measured execution and by every frame for deadline extension. Function frames are more
expensive: `push` performs a `FastMap::entry` for the call site, can allocate on its first call, and
`pop` performs another lookup and updates another set of counters
([recorder.rs](../../src/pine/profile/recorder.rs#L220)). Function-heavy scripts therefore receive
neither an accurate overhead estimate nor conservative deadline compensation.

Attribution is also subtler than the UI says. The start clock is read after much of `push`, and the
end clock is read before much of `pop`; top-level hook bookkeeping falls into “Not in any line,”
while child cleanup can fall into its parent's exclusive time. Yet the Profile tab says the full
estimated cost is “inside every time,” and describes unattributed time only as bar setup, commits,
and orders ([profile_marks.rs](../../src/pine_studio/profile_marks.rs#L387)). On runs whose measured
slowdown is documented as 20–49%, that distinction is large enough to mislead performance analysis.

Recommendation: either calibrate and count statement and function/call-site paths separately, or
label the figure explicitly as a rough statement-hook estimate. Use a conservative cost for deadline
extension so profiling does not create timeouts that the plain run would avoid. Describe
unattributed time as including profiler bookkeeping, and add a function-heavy calibration test or
benchmark assertion alongside the existing statement-heavy one.

### M4 — Failure freshness differs between the Profile tab and Problems list

`Profiler::status` considers a failure current only when the entire `ProfileKey` matches
([profiler.rs](../../src/pine_studio/profiler.rs#L342)). `Profiler::failure`, which feeds the Problems
tab, editor underline, badge, and chip, checks only script id plus source hash
([profiler.rs](../../src/pine_studio/profiler.rs#L377)). After an input, symbol, timeframe, or data
change, the Profile tab can become `Idle`/`Stale` while the old runtime error remains presented as a
current problem on the unchanged source. The changed context may make the error impossible to
reproduce.

`text_changed` compounds the inconsistent naming by comparing the full key, despite being called
for text recovery; a bar/input/chart change can clear and retry a compile/library refusal whose text
did not change ([profiler.rs](../../src/pine_studio/profiler.rs#L332)).

Recommendation: separate `same_source` from `same_execution` explicitly. Current Problems should
normally require `same_execution`; if retaining same-source failures is useful, show them in a
labelled historical section rather than as current diagnostics. Restrict automatic retry of compile
and library refusals to source/script-type changes. Test an input-dependent runtime failure followed
by an input fix and a data-dependent failure followed by a new candle.

## Low findings

### L1 — Enabled profiling performs avoidable work every frame

While the profiler is on, `profiler_frame` rebuilds the key every frame
([profiler_host.rs](../../src/pine_studio/profiler_host.rs#L43)). That clones the script id and chart
label, hashes the entire source, allocates and sorts all input entries, formats their `Debug`
representation into a new string, and hashes it ([profiler.rs](../../src/pine_studio/profiler.rs#L45)).
This continues indefinitely after a profile is ready.

Recommendation: use cached source/input/chart/data revisions maintained where mutations happen.
This naturally aligns with H1 and avoids making stronger correctness require hashing every candle
twice per frame. Add an allocation-count or revision test showing an unchanged ready profiler does
no identity allocation.

### L2 — Stale same-text results are not dimmed as documented

The result view derives `navigable` solely from whether line numbers still match and passes that
boolean as the `current` colour flag to totals and detail rows
([profile_ui.rs](../../src/pine_studio/profile_ui.rs#L324)). A stale result caused by inputs/data is
still navigable, so most figures remain in primary colours even though the banner says it is stale
and the documentation says stale figures are dimmed. Navigation and freshness are separate
properties and should not share one boolean.

Recommendation: pass `navigable` and `current` separately. Keep rows clickable for same-text stale
profiles while applying a consistent fade/secondary palette to every stale figure, label, bar, and
icon. Add ready/stale render assertions; the existing drawer-size test covers only states without a
profile result.

### L3 — Duration and percentage formatting has visible boundary artifacts

`duration` selects a unit before rounding, so `999_999 ns` becomes `1000 µs` and
`999_999_999 ns` becomes `1000 ms` instead of promoting to the next unit
([profile_marks.rs](../../src/pine_studio/profile_marks.rs#L484)). The test currently locks the first
artifact in. `percent` renders every value at or above 99.95% as `100%`, even when the share is not
actually complete ([profile_marks.rs](../../src/pine_studio/profile_marks.rs#L470)); that contradicts
the test's “do not round up to certainty” name and can hide the profiler's small unattributed share.

Recommendation: round first and promote when the rounded value reaches 1000; show `100%` only for an
exact/clamped share of one and otherwise retain a decimal (or use `<100%`). Cover every unit boundary
and 99.94/99.95/99.99/100% explicitly.

## Additional observations and strengths

- The hook gate remains opt-in, cold, and outside expression evaluation. The statement,
  function-call, and bar hooks are visibly paired on ordinary return and diagnostic paths.
- `RegionTable` is immutable and source anchored, and the recorder keeps aggregates in bounded
  arrays/maps rather than retaining per-execution events.
- The stepping clock is an excellent design choice. Normal-run tests assert exact inclusive,
  exclusive, recursion, line aggregation, branch/loop, library, waiting, and total-time behavior.
- Plain/profiled equivalence tests cover seven representative workloads and separately compare
  strategy trades and alerts.
- Profiling runs on an isolated thread with cooperative cancellation; dropping the UI session
  requests cancellation and the session does not mutate preview/chart state.
- Waiting for `request.security()` data is explicitly separated from script time, with both unit and
  slow-provider coverage.
- The engine/UI boundary is clean: reports and marks are computed once, while editor painting does
  not sort, measure, or format profile data each frame.
- Documentation drift should be corrected with the fixes. It currently says failed reports leave
  out the failing bar, while tests include it; says stale tab figures are dimmed, while same-text
  stale results are not; and says the toolbar is blue while the implementation correctly avoids a
  blue active button.

## Test and validation gaps to close

The current suite is broad in the common path, but the production-readiness fixes need focused
coverage for:

- same-timestamp OHLCV mutations, realtime-to-closed transitions, theme changes, and same-ticker
  provider/source changes;
- visible-range and effective-bar-seconds parity with the normal preview;
- pre-cancel, zero-deadline, between-bar cancellation, and mid-bar timeout semantics;
- a panic with open statement/function frames and invariant validation after recovery;
- statement-heavy versus function-heavy overhead/deadline calibration;
- failure freshness after input/data/chart changes;
- a completed Profile drawer result at all five constrained sizes, ready and stale, not only empty
  states;
- formatted duration and percentage boundaries;
- a real-window pass for tooltip placement, scaling, cancellation feedback, stale fading, and
  keyboard navigation.

## Verification performed

- `cargo test --lib pine::profile --no-fail-fast`  
  Result: **43 passed, 0 failed**.
- `cargo test --lib pine_studio::profile --no-fail-fast`  
  Result: **32 passed, 0 failed, 1 ignored** (the ignored test is the image gallery).
- `cargo clippy --lib -- -D warnings`  
  Result: **passed with no warnings**.

Not run: the full default suite, the optional Pine corpus, ignored profiler benchmarks/gallery,
live network/provider checks, or a manual app session. The working tree already contained unrelated
untracked review files; they were not modified.

## Recommended remediation order

1. Introduce one shared, complete preview execution revision and use it in profile configuration and
   staleness (H1).
2. Freeze and pass visible range/effective timeframe context through UI and AI profiling (H2).
3. Define attempted/partial/completed bar semantics and make interruption/failure copy match them
   (M1).
4. Make bar finalization unwind-safe and enforce the accounting invariant without clamping (M2).
5. Correct overhead calibration/wording and make deadline compensation conservative (M3).
6. Unify failure freshness and automatic retry rules (M4).
7. Replace per-frame key construction with cached revisions, then finish stale-state rendering and
   formatting polish (L1–L3).

## Fix status

### M1 — fixed (#320)

Confirmed, with one correction to the finding: counting the *failing* bar on a runtime error is
deliberate (its time is real and the report keeps what ran), the tests pin it, and the documentation
and a comment were wrong. The behaviour stays; the wording changed.

- `Session::execute` checks for cancellation and an expired deadline **before** the recorder opens a
  bar (hooked sessions only, so the plain statement loop is untouched). A run stopped before its
  first statement counts no bar and no progress.
- `end_bar` takes a `BarOutcome` (`Skipped`, `Completed`, `Cut`) instead of a boolean. A `Cut` bar
  (cancelled, out of time, or failed part-way) is counted and in the total, so the accounting
  identity holds, but the progress counter advances only for completed bars and
  `Summary::cut_bar` records it.
- Copy: “cancelled during bar N” when the last bar was cut, “cancelled after N bars” otherwise, and
  “the script failed on bar N” (Profile tab, Problems, AI `profile` tool). `docs/pine-profiler.md`
  says the same.
- Tests: pre-raised cancellation flag, zero deadline, cancellation between bars, a timeout inside a
  long bar.

### M2 — fixed (#320)

Confirmed at the engine level. Today the Pine Studio discards a failed report, so no screen showed the
inconsistency, but the engine and the AI layer received it.

- The recorder remembers that a bar is open. `finish` closes such a bar like a cut one: its time joins
  `total_ns`, it is not counted among the bars, and `cut_bar` is set. The silent `min(attributed,
  total)` repair is gone and replaced by a `debug_assert!`.
- Chosen policy (the review offered two): count the elapsed time and label the bar cut, rather than
  discard its counters, because the regions that ran in it are real.
- No RAII guard in `session.rs`; that keeps the hot path as it was.
- Tests: a recorder finished with an open nested function frame, and a session whose
  `request.security` provider panics.

### M3 — fixed (#320)

Confirmed. The recorder now calibrates a statement pair and a function-call pair separately
(`Summary::pair_cost_ns` and `call_pair_cost_ns`), counts each kind of frame, and uses the same split
for the overhead estimate and for the deadline extension. The pairs timed per round were halved so the
two calibrations cost about what one did.

- The copy no longer says the cost is “inside every time”: it says the estimate is per statement and
  per call, mostly inside the line measured, and that “Not in any line” also holds part of the
  profiler's bookkeeping. The AI tool's text says the same.
- Not changed: where the clock reads fall inside `push`/`pop`. Moving them would trade one bias for
  another; the documentation now describes it instead.
- Test: a call-only run is charged at the call cost, and the deadline receives the same figure.

### H1, H2, M4, L1 — fixed (#321, stacked on #320)

**H1** confirmed. The key is now `(script, source, RunIdentity)`, all numbers. `RunIdentity`
(`PreviewState::run_identity`) covers a fingerprint of every field of every candle, whether the last
candle has closed, the ticker, data source and timeframe, the theme, the inputs, and the visible
range for a script that reads it. The candle fingerprint is `ui::candles_fingerprint`, extracted from
the chart's `run_revision`, so the chart and the profiler share one definition. Tests: a changed close
on an unchanged timestamp, a changed close time, a changed volume, a candle closing, the theme, the
same ticker on another source, an input.

**H2** confirmed. `DebugFeed` carries `visible_range` (from the new `ChartCard::visible_open_range`,
which `pine_outputs` now uses too); `profiler_start` and the AI `profile` tool pass it into
`ProfileConfig`. End-to-end test: a script branching on both builtins is profiled over exactly the
candles on screen, fewer than the series. Differences from the recommendation: the effective bar
seconds are not added, because the chart passes none for plain candles (`source_bar_seconds` is `None`
unless the bars were constructed) and the preview draws plain candles; and the debugger's own start
still ignores the range (it shares `DebugFeed`, so it is a one-line follow-up, but it is outside this
review).

**M4** confirmed. `ProfileKey` now has `same_text` and `same_execution`. The Problems list, underline,
badge and chip (`Profiler::failure`) require `same_execution`; `text_changed` retries a compile or
library refusal only when the text changed, so a new candle no longer clears and repeats it; the
Profile tab keeps showing a refusal while the text is the same. Tests: an input-dependent failure
stops being a problem when the input changes; a refusal survives a new candle and is retried on new
text.

**L1** partly fixed. Building the key no longer clones strings, sorts, or formats `Debug`; the candle
fingerprint is made when candles arrive, not per frame, and the inputs hash order-independently
without allocating. The script text is still hashed once per frame while the profiler is on (an FNV
pass over the source), which I judged not worth a revision counter on the editor. There is no
allocation-count test; `ProfileKey` being `Copy` is asserted instead.

A consequence worth knowing: a live preview whose last candle changes on each refetch now marks a
profile stale each time, which is what H1 asked for.

### L2, L3 and documentation drift — fixed (#322, stacked on #321)

**L2** confirmed. The result view had one boolean (`lines_match`) for two facts. `Freshness
{ navigable, current }` separates them: rows stay clickable while the text matches, but a stale
profile's totals, icon chips, heat bars, labels and figures all fade. The drawer test now draws a
finished profile, ready and stale (same text and other text), at the five drawer sizes, and a test
pins the mapping from status to freshness. I also rendered the ready and stale pictures and looked
at them.

**L3** confirmed. `duration` rounds to three figures and then picks the unit (`999,999 ns` is
`1.00 ms`; `99.96 µs` is `100 µs`); `percent` prints `100%` only for a whole share and caps anything
less at `99.9%`. The test that locked `1000 µs` in was replaced, with every unit boundary and
99.94 / 99.95 / 99.99 / 100% covered.

**Documentation drift** fixed in `docs/pine-profiler.md`: the failing bar (#320), stale dimming (now
true), and the toolbar colour (the code uses the primary text colour while on, so the doc was wrong,
not the code).

## Not done

- A real-window pass (tooltip placement near the window edge, UI scales, keyboard navigation,
  cancellation feedback). Everything above was checked headlessly and in rendered pictures.
- The debugger's own start still ignores the preview's visible range (it shares `DebugFeed`).
- The script text is still hashed once per frame while the profiler is on (L1, partial).
- No allocation-count test for the key; `ProfileKey` being `Copy` is asserted instead.
