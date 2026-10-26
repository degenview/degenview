# Implement a Production-Ready Pine Script Profiler in DegenView

## Objective

Implement a complete, high-performance **Pine Script Profiler** in DegenView's existing Pine engine and Script Manager.

The profiler must allow users to identify exactly which parts of their Pine scripts consume the most execution time, how frequently those parts execute, and where performance bottlenecks occur.

The experience should be comparable to TradingView's Pine Profiler in terms of functionality, usability, accuracy, and visual clarity, while fitting naturally into DegenView's existing native Rust/egui architecture.

**Use the attached TradingView screenshot as a reference for the profiler's visual presentation and interaction model, but not as an exact design specification.** Follow DegenView's existing design system and make thoughtful improvements where appropriate.

This is not a prototype. The final implementation must be polished, performant, maintainable, thoroughly tested, and production-ready.

### Non-negotiable requirements

1. Integrate directly into the existing Pine compiler, runtime, execution controller, and Script Manager.
2. Identify slow individual lines, expressions, loops, functions, and code blocks.
3. Display accurate execution counts, elapsed time, and relative performance costs.
4. Present profiling results directly alongside the corresponding source code.
5. Keep profiling overhead effectively zero when disabled.
6. Ensure profiling does not change script behavior or output.
7. Support indicators, strategies, libraries, and the Pine language constructs already implemented by DegenView.
8. Handle loops, nested scopes, functions, conditional branches, and repeated execution correctly.
9. Maintain responsive UI behavior while profiling expensive scripts.
10. Do not introduce a second Pine interpreter, duplicate execution pipeline, or unrelated profiling subsystem.
11. Do not use fragile source-code rewriting, string matching, arbitrary timing estimates, or UI-side approximations.
12. Deliver complete functionality with meaningful automated tests and performance benchmarks.

---

# 1. Research TradingView's Pine Profiler

Before implementation, research TradingView's official documentation and other reliable technical resources to understand how Pine profiling works.

Primary reference:

https://www.tradingview.com/pine-script-docs/writing/profiling-and-optimization/

Investigate:

- How the Pine Profiler is enabled and disabled.
- How results appear in the Pine Editor.
- How execution time is attributed to source lines.
- How code blocks are represented.
- How loop execution is measured.
- How execution counts are calculated.
- How profiling handles user-defined functions.
- How time percentages are calculated.
- How total execution time is presented.
- How users inspect detailed profiling results.
- How the interface highlights expensive code.
- How results change when scripts are edited or rerun.
- How TradingView handles compiler optimizations and repeated expressions.
- How profiling affects execution performance.

Study the attached screenshot carefully.

It illustrates several useful interactions:

- A performance gutter beside the editor.
- Horizontal bars representing relative execution cost.
- Percentage labels.
- Flame icons identifying expensive sections.
- Source-code highlighting.
- Hover tooltips with detailed measurements.
- Code-block ranges.
- Execution counts.
- Line time versus code-block time.

For example, the screenshot displays information similar to:

    Code block range: [221; 255]
    Time: 56.7% (0.8s from 1.5s total)
    Line time: 22.3% (329ms from 1.5s total)
    Executions: 5460

Understand the distinction between these measurements before designing the underlying data model.

**Do not blindly reproduce TradingView's numbers or assume its profiler's internal measurement methodology is publicly documented.**

Where TradingView's implementation details are unknown, choose a technically sound approach, document its semantics, and make the results understandable to users.

---

# 2. Audit DegenView's Existing Architecture

Inspect the actual repository before modifying anything.

The existing architecture includes a custom Rust Pine compiler and runtime, a Script Manager, chart integration, indicators, strategies, alerts, and execution controls.

Relevant areas may include:

    src/pine/
    src/pine/compiler.rs
    src/pine/lexer.rs
    src/pine/parser.rs
    src/pine/type_checker.rs
    src/pine/program.rs
    src/pine/outputs.rs
    src/pine/broker.rs

    src/pine_host.rs
    src/pine_subscription_host.rs
    src/model/pine_instance.rs

    src/script_manager/
    src/script_manager/model.rs
    src/script_manager/store.rs

    src/ui/pine_report.rs
    src/ui/pine_settings_popover.rs
    src/ui/pine_style_view.rs

    src/workspace.rs
    src/ui/chart_card.rs
    src/ui/theme.rs
    src/ui/kit.rs

These are architectural starting points, not assumptions about the current repository. Locate the actual implementations and follow their established conventions.

Determine:

- How Pine source is parsed and compiled.
- How AST nodes or intermediate instructions map to source locations.
- How the runtime evaluates expressions and statements.
- How loops and function calls execute.
- How scripts run across historical bars.
- How realtime bars are processed.
- How execution limits and timeouts work.
- How script compilation and execution errors are reported.
- How script instances are associated with charts.
- How the Script Manager manages editor state.
- How syntax highlighting and line numbers are rendered.
- How scripts are applied, edited, saved, and rerun.
- How background execution and cancellation currently work.
- How profiling results can be delivered to the UI without blocking it.

**Reuse existing source-location metadata, runtime execution infrastructure, diagnostics, and UI components wherever possible.**

Refactor shared code only when doing so produces a genuinely cleaner architecture.

---

# 3. Profiler Architecture

Build the profiler as an optional instrumentation layer within the existing Pine execution engine.

Conceptually:

    Pine Source
        |
        v
    Lexer / Parser
        |
        v
    Compiler
        |
        +---- Source Location Metadata
        |
        v
    Compiled Program
        |
        +---- Normal Execution
        |
        +---- Profiled Execution
                  |
                  v
             Timing Events
                  |
                  v
             Aggregation
                  |
                  v
             Profile Result
                  |
                  v
             Script Manager UI

The normal execution path must remain the authoritative execution path.

The profiled path must execute the same program with additional measurement instrumentation.

## 3.1 Profiling must be optional

The profiler should be disabled by default.

When disabled:

- No per-line timers.
- No per-expression timing.
- No profiling event allocation.
- No profiling map lookups inside hot loops.
- No profiler-specific locks.
- No profiling-related UI updates.
- No unnecessary source-location processing during runtime execution.

Prefer a design that selects the appropriate execution mode outside the hottest runtime loops.

Possible approaches include specialized execution paths, generic instrumentation policies, or a carefully designed runtime mode.

Evaluate the tradeoffs against the actual interpreter architecture.

Avoid repeatedly checking a profiling-enabled boolean for every expression if a better design can eliminate that cost.

The goal is **no measurable performance regression in normal execution**, within benchmark noise.

## 3.2 Source-location identity

Profiling data must map reliably to source code.

Use compiler-generated source spans or stable instruction identities.

Every measurable execution region should have an appropriate source association.

Support:

- Single-line expressions.
- Multiple statements on one line.
- Multiline expressions.
- Function definitions.
- Function calls.
- Loop bodies.
- Conditional blocks.
- Nested scopes.
- Library code.
- Repeated execution of the same source location.

Do not associate profiling results with source text through string matching.

Do not depend solely on line numbers when multiple executable regions occupy the same line.

Line numbers should be a presentation layer over precise compiler source locations.

## 3.3 Typed profiling model

Create a clean, testable profiling model.

Conceptually:

    ProfileSession
    ProfileRegion
    ProfileRegionKind
    ProfileSample
    ProfileResult
    ProfileSummary
    ProfileExecutionContext

Possible region kinds:

    Statement
    Expression
    Function
    FunctionCall
    Loop
    Conditional
    CodeBlock
    Script

Possible measurements:

    execution_count
    inclusive_time
    exclusive_time
    total_time
    average_time
    minimum_time
    maximum_time
    source_span
    parent_region
    child_regions

These are conceptual examples. Adapt the final model to the compiler and runtime.

Do not store redundant measurements when they can be safely derived.

Avoid excessive memory allocation and unbounded sample collection.

Prefer aggregated counters over retaining every execution event.

---

# 4. Accurate Timing and Attribution

Timing correctness is one of the most important parts of this feature.

Use a monotonic, high-resolution clock suitable for Rust profiling.

Avoid wall-clock timestamps for elapsed-time measurement.

## 4.1 Inclusive and exclusive time

Support two clearly defined timing concepts.

**Inclusive time:** Time spent executing a region, including its nested child regions.

**Exclusive time:** Time spent executing a region itself, excluding measured child regions.

For example:

    for i = 0 to 1000
        a = expensive_function(i)
        b = a * 2

The loop's inclusive time includes execution of its body.

The function's inclusive time includes its internal operations.

The loop's exclusive time should not double-count nested measured regions.

Do not sum overlapping inclusive measurements to calculate script totals.

Use a properly nested timing model or another mathematically consistent attribution strategy.

## 4.2 Execution counts

Count actual executions, not estimated executions.

Examples:

- A statement executed once on each of 5,000 bars should report 5,000 executions.
- A statement executed 20 times inside a loop on each of 5,000 bars should report 100,000 executions.
- A conditional branch should count only when its body actually executes.
- A function body should count every invocation.
- A function call site should be distinguishable from the function definition where supported.

Handle early returns, breaks, continues, errors, and cancellations correctly.

## 4.3 Timing overhead

Instrumenting small expressions may cost more than the expressions themselves.

Account for this explicitly.

Requirements:

- Measure and document instrumentation overhead.
- Avoid presenting nanosecond-level precision that the profiler cannot justify.
- Distinguish measured elapsed time from estimated or adjusted execution cost.
- Do not silently subtract arbitrary constants from measurements.
- Never produce negative adjusted durations.
- Avoid claiming that profiling time equals normal execution time.

If calibration or sampling is used, document the methodology and limitations.

Prefer reliable, interpretable measurements over misleading precision.

## 4.4 Script-level total time

Define a consistent profiling denominator.

The UI must distinguish, where applicable:

- Total profiled script execution time.
- Time attributed to source regions.
- Runtime overhead not attributed to a source region.
- Profiler instrumentation overhead.
- Compilation time, which should not be mixed into execution measurements.

Percentages must be calculated consistently.

A displayed 57% should have an unambiguous meaning.

---

# 5. Loops, Functions, and Code Blocks

The profiler must handle complex Pine programs correctly.

## 5.1 Loops

Support:

- For loops.
- While loops.
- Nested loops.
- Loop conditions.
- Loop bodies.
- Break and continue.
- Loops spanning multiple source lines.

The profiler should identify expensive loops even when no individual line inside the loop appears particularly expensive.

Example:

    for i = 0 to 10000
        value = array.get(values, i)
        total += value

The UI should reveal:

- Total time spent in the loop.
- Number of times the loop statement was entered.
- Execution counts for statements inside the loop.
- Time spent in individual statements.
- The loop's source range.

Do not confuse loop entry counts with total body iteration counts.

If iteration counts are exposed, store them as a separate metric.

## 5.2 Functions

Support user-defined functions, nested calls, recursion where the runtime permits it, and functions called from multiple locations.

The profiler must distinguish between:

- Time spent at a call site.
- Time spent inside the called function.
- Time spent in nested calls.

Avoid double-counting.

A function called from three locations should have a meaningful aggregated function-level result.

Where practical, also provide per-call-site attribution.

## 5.3 Conditional execution

Handle:

- If/else.
- Ternary expressions.
- Short-circuit operators.
- Switch expressions.
- Conditional function calls.

Only measure code that actually executes.

## 5.4 Built-in functions

The profiler should attribute expensive built-in Pine operations to the corresponding source expressions.

Examples:

    ta.sma(close, 200)
    ta.highest(high, 500)
    request.security(...)
    array.sort(values)

Measure the actual synchronous work performed by the runtime.

Do not misleadingly attribute unrelated network waiting, asynchronous processing, or external data acquisition to a source expression.

If a built-in operation has separate execution phases, make attribution consistent and document the boundaries.

---

# 6. Script Manager Integration

The profiler must feel like a native part of DegenView's Script Manager.

Do not create a disconnected profiling window or separate source-code viewer.

## 6.1 Profiler controls

Add a clearly discoverable profiler control to the Script Manager's existing toolbar.

Suggested states:

    Profiler Off
    Profiling...
    Profile Ready
    Profile Stale
    Profile Failed

The control should:

- Enable profiling.
- Disable profiling.
- Show whether profiling is active.
- Indicate when results are being collected.
- Allow the user to rerun profiling.
- Provide access to profiling results.

Follow existing toolbar conventions for icons, spacing, hover states, tooltips, and keyboard accessibility.

Do not introduce a new visual language.

## 6.2 Editor integration

Add a profiling gutter alongside the existing line-number gutter.

The profiling gutter should display performance information for relevant lines.

Use the screenshot as inspiration.

Suggested visual elements:

- Horizontal performance bars.
- Execution-time percentages.
- Subtle heat indicators.
- Flame icons for especially expensive regions.
- Hoverable measurement labels.
- Source-region highlighting.

The user should be able to scan a script and immediately understand where execution time is concentrated.

Avoid excessive visual noise.

Lines without meaningful profiling data should remain visually quiet.

## 6.3 Performance bars

Display relative execution cost using horizontal bars.

Requirements:

- Scale consistently across the current profile.
- Clearly distinguish high-cost and low-cost lines.
- Avoid misleading normalization.
- Support very small percentages.
- Keep labels legible.
- Work in dark and light themes.
- Respect the application's existing accent colors.

Consider using a warm accent for expensive regions, as shown in the reference, but integrate it with DegenView's palette.

Do not hardcode colors or dimensions throughout the rendering code.

## 6.4 Hover tooltips

Hovering a profiling bar or highlighted source region should display detailed information.

For example:

    Code block: Lines 221–255

    Total time: 800 ms
    Share of script: 56.7%

    Line time: 329 ms
    Share of script: 22.3%

    Executions: 5,460

Additional useful metrics may include average time per execution and inclusive/exclusive timing.

Only show metrics whose semantics are reliable.

Use DegenView's existing tooltip or popover infrastructure.

Tooltips must:

- Appear promptly.
- Remain stable while the pointer moves.
- Avoid flickering.
- Stay within window bounds.
- Work at different UI scales.
- Avoid obscuring too much source code.
- Display sensible units and precision.

## 6.5 Source highlighting

Highlight source ranges associated with expensive operations.

For example, hovering a loop's performance indicator should highlight the corresponding loop body.

Requirements:

- Correctly handle multiline regions.
- Respect existing syntax highlighting.
- Preserve text selection.
- Preserve search highlighting.
- Preserve compiler diagnostics.
- Avoid hiding error underlines.
- Keep highlighting aligned during scrolling.
- Work with wrapped lines, if the editor supports them.

The profiler must not interfere with normal code editing.

## 6.6 Profile summary

Provide a compact summary of the current profiling session.

Useful information:

- Total execution time.
- Number of bars processed.
- Total measured executions.
- Most expensive line or region.
- Most expensive function.
- Profile status.
- Profiling scope.

Where useful, offer a ranked view of the most expensive regions.

Selecting an entry should navigate directly to its source location.

Avoid introducing a large dashboard unless it provides clear additional value over the inline editor visualization.

---

# 7. Profiling Lifecycle

Profiling must integrate with the actual script execution lifecycle.

## 7.1 Starting profiling

When the user enables profiling:

1. Resolve the intended script and execution context.
2. Compile or reuse the appropriate compiled program.
3. Initialize an isolated profiling session.
4. Execute the script using the instrumented runtime.
5. Aggregate results.
6. Publish the completed profile to the Script Manager.
7. Display source annotations.

Do not assume an editor's selected script is necessarily the same revision as a script already applied to a chart.

Make this distinction explicit.

## 7.2 Source edits

Profiling results belong to a specific source revision.

When the user changes source code:

- Mark previous results stale.
- Do not attach old measurements to unrelated new lines.
- Do not silently display stale percentages as current.
- Allow profiling to be rerun.
- Preserve normal editor responsiveness.

Use a stable source revision or content fingerprint.

## 7.3 Script inputs and chart context

A profile must be associated with the context in which it was produced.

Consider:

- Script source revision.
- Input parameters.
- Symbol.
- Timeframe.
- Chart type.
- Historical data range.
- Number of bars.
- Runtime configuration.
- Relevant library revisions.

Changing a parameter that affects execution should invalidate or replace the corresponding profile.

## 7.4 Multiple script instances

Two instances of the same script may have different inputs or execute against different charts.

Do not mix their profiling results.

Use the existing script-instance identity and execution context.

## 7.5 Cancellation and errors

Support:

- User cancellation.
- Script runtime errors.
- Compilation failures.
- Execution timeouts.
- Chart closure.
- Script deletion.
- Source changes during profiling.
- Application shutdown.

Never leave a profiler session permanently stuck in a running state.

Incomplete results must not be presented as complete.

Where useful, allow explicitly labeled partial results.

---

# 8. Historical Bars, Realtime Execution, and Replay

Pine scripts execute across historical and realtime data.

The profiler must respect those semantics.

## 8.1 Historical execution

A full profiling run should measure execution across the selected historical dataset.

Display the number of bars processed.

Do not profile only the currently visible chart viewport unless the user explicitly requests such a scope.

Use the same data and execution semantics as the existing Pine runtime.

## 8.2 Realtime execution

Define a clear policy for realtime updates.

Avoid continuously accumulating results indefinitely.

Prefer bounded, understandable profiling sessions.

If realtime profiling is supported, distinguish it from historical profiling and provide a clear reset mechanism.

## 8.3 Bar replay

Profiling during replay must respect the replay cursor.

Do not expose future bars.

Do not allow profiling to advance replay or mutate replay state.

A profile generated for one replay position must not silently appear valid for another.

---

# 9. Strategies, Indicators, and Alerts

The profiler must work with the Pine script types supported by DegenView.

## Indicators

Profile calculations, loops, function calls, and supported plotting operations.

## Strategies

Profile strategy calculations and runtime operations without changing strategy results.

Profiling must not alter:

- Order placement.
- Fill calculations.
- Position state.
- Equity curves.
- Trade reports.
- Strategy execution order.

## Alerts

Profiling must not cause duplicate alert notifications or webhook deliveries.

A profiling run must not create external side effects merely because the script is being evaluated for measurement.

Prefer a side-effect-safe execution context using existing runtime abstractions.

Do not modify live alert subscriptions or their deduplication state.

---

# 10. Threading and UI Responsiveness

Profiling expensive scripts must not freeze the Script Manager or chart UI.

Follow DegenView's existing background-work architecture.

Requirements:

- Run expensive profiling work off the UI thread where appropriate.
- Use bounded worker concurrency.
- Support cancellation.
- Avoid holding application-wide locks during execution.
- Deliver results through existing thread-safe communication mechanisms.
- Reject stale results when a newer session supersedes them.
- Avoid excessive repaint requests.
- Avoid blocking chart rendering while collecting metrics.

Do not introduce a new application-wide asynchronous runtime merely for profiling.

If the existing Pine execution architecture requires refactoring to support safe background profiling, implement that refactoring cleanly.

Preserve the application's established ownership and synchronization rules.

---

# 11. Performance and Memory Requirements

Profiling must be efficient enough for realistic Pine scripts.

## Disabled overhead

This is a critical acceptance requirement.

Benchmark normal execution before and after the implementation.

Use representative workloads:

- Simple indicators.
- Heavy mathematical calculations.
- Large historical datasets.
- Nested loops.
- Repeated function calls.
- Array-heavy scripts.
- Strategies.
- Scripts with many source lines.

The disabled profiler should produce no statistically meaningful performance regression.

Do not claim zero overhead without measurements.

## Enabled overhead

Profiling necessarily introduces measurement cost.

Optimize that cost without compromising correctness.

Prefer:

- Compact region identifiers.
- Contiguous storage where appropriate.
- Preallocated counters.
- Efficient source-location mappings.
- Bounded memory usage.
- Minimal hot-path allocations.
- Aggregation outside the UI.
- No per-execution logging.

Avoid a global mutex around every profiling event.

Avoid storing millions of individual timing records when aggregate counters are sufficient.

## Resource limits

Respect existing Pine runtime limits.

The profiler must not bypass:

- Execution timeouts.
- Memory limits.
- Loop limits.
- Recursion limits.
- Cancellation.
- Existing runtime safety checks.

Account for profiling overhead when interpreting execution timeouts so that enabling profiling does not silently create inconsistent runtime behavior.

---

# 12. Visual Design and Polish

The profiler should feel like it has always belonged in DegenView.

Follow existing:

- Typography.
- Spacing.
- Corner radii.
- Color tokens.
- Toolbar dimensions.
- Hover animations.
- Tooltip styling.
- Selection colors.
- Scroll behavior.
- Keyboard navigation.
- Accessibility conventions.

The screenshot demonstrates a useful visual hierarchy:

1. Source code remains the primary content.
2. Performance bars occupy a narrow gutter.
3. Expensive regions stand out.
4. Detailed information appears on demand.
5. Code-block highlighting connects metrics to actual source.

Preserve that hierarchy.

Avoid making the editor feel crowded.

## Additional requirements

- Smooth scrolling with profiling enabled.
- Stable alignment between line numbers, profiling indicators, and code.
- Correct clipping and virtualization for large scripts.
- Retina/high-DPI support.
- Correct behavior during window resizing.
- Correct behavior in detached native windows.
- No tooltip flicker.
- No unnecessary layout jumps.
- No horizontal scrollbar introduced solely by profiler decorations.
- No unnecessary animations during execution.

Use subtle visual feedback.

The profiler should communicate performance information, not compete visually with the code.

---

# 13. Testing

Build comprehensive automated coverage.

## 13.1 Measurement correctness

Test:

- Single statements.
- Sequential statements.
- Nested blocks.
- Nested loops.
- Functions called repeatedly.
- Multiple call sites.
- Conditional branches.
- Early returns.
- Break and continue.
- Recursive calls where supported.
- Multiline expressions.
- Multiple expressions on one line.
- Built-in functions.
- Runtime errors.
- Cancellation.

Verify exact execution counts wherever deterministic.

Verify inclusive/exclusive timing invariants.

Timing tests must use appropriate tolerances or controlled clocks rather than relying on fragile wall-clock thresholds.

## 13.2 Runtime equivalence

Execute the same scripts with profiling enabled and disabled.

Verify identical:

- Indicator outputs.
- Strategy results.
- Series values.
- Plot values.
- Runtime errors.
- Observable execution behavior.

Use deterministic datasets.

The profiler must never change a script's financial calculations.

## 13.3 Source mapping

Test:

- Blank lines.
- Comments.
- Indentation.
- Multiline expressions.
- Nested functions.
- Unicode source text.
- Source edits.
- Compiler transformations.
- Repeated expressions.
- Optimized instructions.

Ensure source annotations remain correct.

## 13.4 UI tests

Test:

- Enable/disable.
- Profiling progress.
- Completed results.
- Stale results.
- Failed results.
- Hover tooltips.
- Code-block highlighting.
- Editor scrolling.
- Source navigation.
- Theme changes.
- Window resizing.
- Keyboard navigation.
- Multiple open scripts.

Use DegenView's existing egui testing infrastructure where possible.

## 13.5 Performance benchmarks

Create reproducible release-mode benchmarks.

Measure:

- Baseline execution before the change.
- Execution after the change with profiling disabled.
- Execution with profiling enabled.
- Profiling memory usage.
- Large-script rendering cost.
- UI responsiveness during profiling.

Report median timings and measurement variability over repeated runs.

Do not use debug-build performance as evidence of release performance.

---

# 14. Implementation Quality

The final code must be production quality.

Requirements:

- Idiomatic Rust.
- Clear ownership boundaries.
- Strong types.
- Small, focused modules.
- Documented timing semantics.
- Explicit lifecycle management.
- No hidden global mutable profiling state.
- No unbounded caches.
- No arbitrary sleeps.
- No polling loops that waste CPU.
- No unnecessary cloning of compiled programs.
- No duplicate interpreter.
- No source-code string rewriting.
- No fake profiling results.
- No placeholder controls.
- No hardcoded demo data.
- No broad exception or error suppression.
- No unrelated architectural rewrites.

Reuse existing infrastructure wherever appropriate.

If the current architecture makes accurate profiling difficult, implement the necessary foundational changes properly rather than adding temporary workarounds.

---

# 15. Implementation Process

Work in the following order.

### Phase 1 — Research and architecture audit

Study TradingView's profiler and DegenView's existing compiler, runtime, and Script Manager.

Document the proposed timing semantics and source attribution model.

### Phase 2 — Profiling instrumentation

Implement optional instrumentation in the existing execution pipeline.

Establish correct timing and execution counts before adding UI.

### Phase 3 — Aggregation and lifecycle

Implement profiling sessions, aggregation, cancellation, revision tracking, and result delivery.

### Phase 4 — Script Manager integration

Implement the toolbar control, profiling gutter, performance bars, source highlighting, tooltips, and summary.

### Phase 5 — Complex execution semantics

Verify loops, functions, nested blocks, built-ins, strategies, and realtime behavior.

### Phase 6 — Performance optimization

Benchmark disabled and enabled execution paths.

Remove unnecessary overhead and allocations.

### Phase 7 — Testing and polish

Complete automated tests, UI validation, release-build performance checks, and edge-case handling.

Do not stop after implementing the instrumentation backend. The user-facing profiler is part of the required deliverable.

---

# 16. Acceptance Criteria

The feature is complete only when all of the following are true:

- [ ] The profiler can be enabled and disabled from the Script Manager.
- [ ] Profiling results map correctly to source code.
- [ ] Slow lines and expensive code blocks are immediately identifiable.
- [ ] Execution counts are correct.
- [ ] Inclusive and exclusive timing are mathematically consistent.
- [ ] Nested loops and function calls are measured correctly.
- [ ] Performance bars and tooltips are polished and readable.
- [ ] Source highlighting works without interfering with editing.
- [ ] Editing source invalidates stale results.
- [ ] Multiple script instances remain isolated.
- [ ] Historical and replay execution are handled correctly.
- [ ] Profiling does not alter script outputs.
- [ ] Profiling does not trigger unintended alerts or trading side effects.
- [ ] Profiling can be cancelled safely.
- [ ] Expensive profiling runs do not freeze the UI.
- [ ] Profiling memory usage is bounded.
- [ ] Disabled profiling introduces no statistically meaningful execution regression.
- [ ] The implementation passes the existing test suite.
- [ ] New profiling tests and benchmarks pass.
- [ ] The UI has been checked in both light and dark themes.
- [ ] No placeholders, hacks, or incomplete feature paths remain.

---

# 17. Final Deliverables

After implementation, provide a concise engineering report containing:

1. **Architecture:** How profiling integrates with the existing Pine compiler and runtime.
2. **Instrumentation:** How execution regions are identified and measured.
3. **Attribution:** How inclusive time, exclusive time, line time, and execution counts are calculated.
4. **UI:** How the profiler integrates with the Script Manager.
5. **Lifecycle:** How profiling sessions, source revisions, cancellation, and errors are handled.
6. **Performance:** Release-mode benchmark results, including disabled overhead.
7. **Testing:** Tests added and their results.
8. **Limitations:** Any genuine measurement limitations or unsupported language constructs.
9. **Files changed:** Summary of the implementation footprint.

Clearly distinguish verified behavior from assumptions.

Do not claim production readiness if important tests, benchmarks, or manual UI checks have not been completed.

## Final Goal

Deliver a **fast, accurate, deeply integrated Pine Script Profiler** that helps DegenView users understand and optimize their scripts.

A user should be able to enable profiling, execute a script, immediately see which lines and code blocks consume the most time, hover for precise execution statistics, and navigate directly to performance bottlenecks.

The experience should feel comparable in quality and usability to TradingView's Pine Profiler, while remaining native to DegenView.

**Prioritize measurement correctness, effectively zero disabled overhead, architectural cleanliness, and excellent UI polish. No shortcuts, no fragile instrumentation, and no half-finished implementation.**