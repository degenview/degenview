# Pine and Pine Studio guidance

## Runtime

- Pine engine stays under `src/pine/`; it and Pine domain types in `model/` are pure (no egui
  types). Alerts and subscriptions compile the same modules.
- Unsupported behavior is never silent: stand-ins emit `ignored_features` warnings
  (`PINE7001`–`PINE7006`) and live in `misc_builtins`; object results or feeds without one stay
  `PINE9003` errors.
- `log.*()` goes to `Session.logs`, never `RuntimeState` (rollback would discard it). Opt-in
  (`ExecutionController::with_logs`); readers apply `LogBatch`es in order via `LogHistory`. Limits in
  `pine::logs`; read [Pine Logs](../pine-logs.md) before changing them.
- Debugger (`pine/debug/`, [Pine debugger](../pine-debugger.md)) is opt-in: `Session::debug` is
  `None` unless `attach_debugger` ran; hooks (statement loop, `invoke`, `execute_bar`,
  `request.security` evaluator) test it and call `#[cold] #[inline(never)]` helpers. Never hook
  `eval`, `budget` or any per-expression path, or add debugger-less work to the statement loop.
  Check `runtime/session/` changes with `tools/pine-bench.sh <base-rev>` (interleaved A/B;
  checksums prove both builds ran the same program). Watches/conditions must parse with
  `pine::debug::parse_expression`; its allow-list takes only calls that neither write state nor
  allocate a handle, tested against a clean run.
- Profiler (`pine/profile/`, [Pine profiler](../pine-profiler.md)) shares the gate: `Session::hooks`
  is `None` unless `attach_debugger` or `attach_profiler` ran (one or the other); statement loop,
  `invoke` and `execute` test it and call `#[cold] #[inline(never)]` helpers.
  - Keep the statement loop's shape (block-level test plus second loop measured 1–2% slower in
    layout alone).
  - Every `hook_before` needs `hook_after`/`hook_failed`; every `hook_enter_function` needs
    `hook_leave_function`.
  - Region = statement or user function, keyed by its parser anchor offset; library code
    (`context.scope`) is never timed.
  - Keep `Σ line time + not attributed = total` exact (tests assert it on a stepping clock).
  - `ProfileKey` (text plus `PreviewState::run_identity`) must cover all a script sees in the preview
    run; a new chart-run input (`host::pine::chart::ChartRuns::update`, `ChartFrame`) goes into the identity and
    `DebugFeed`.
  - Bench: `tools/pine-bench.sh <base-rev> <head-rev>` (head must be a commit; `profiled_*`
    workloads = recorder cost).
- `request.footprint()` handles are stateless (footprint = bar index; `volume_row` = bar index +
  row); data in `Session::footprints`, outside `RuntimeState` so rollback does not copy it. Read
  [Pine volume footprints](../pine-footprint.md) before changing the algorithm: mostly inferred.
- `ta.*` and a missing (`na`) source bar, as measured on TradingView (Binance bars, probes P03/P09):
  `ta.ema`, `ta.rma`, `ta.macd` and the legs of `ta.kc`, `ta.tsi` and `ta.rsi` are `extra::Derived`
  averages: a `na` bar gives `na`, leaves the state alone and the average starts as the mean of the
  first `length` values that exist. `ta.rsi` is `na` while `ta.change(src)` is (the `na` bar and the one
  after). `ta.stdev`, `ta.variance`, `ta.median`, `ta.range`, `math.sum` and the stdev of `ta.bb` /
  `ta.bbw` skip the `na` bar and keep their value there (`last_valid`). `ta.wma` is `na` on the bar and
  later windows hold the last value before it in its slot. `ta.dev` stays strict (TradingView is,
  against its own manual). **Not measured, still strict** (`na` for `length` bars after a gap):
  `ta.cog`, `ta.cmo`, `ta.cci`, `ta.hma`, `ta.correlation`, `ta.mode`, `ta.mfi`, `ta.vwma`,
  `ta.wpr`, `ta.rci`, `ta.highestbars`, `ta.lowestbars`, `ta.rising`, `ta.falling`; their TradingView
  rule is a known divergence until a probe settles it. Never add a per-bar growing field to
  `ta::State` for this (cloned on every realtime tick).
- Execution-model gaps: [the audit](../reviews/pine-execution-model-audit.md); read it before relying
  on rollback exceptions. Still open: broker orders do not survive a rolled-back tick; runtime errors
  do not halt the script.
- Function locals (`Session::invoke_with`): the variable table is flat. An ordinary local hides an
  outer variable of its name only from its declaration on: a block's to the end of that execution of
  the block, a top-level one to the end of the call (the call's `saved`). Which block declarations
  hide something is decided statically: `scopes::mark_shadowing` (at session build, for the script
  and its libraries) flags an ordinary declaration in a block whose name was a global, local,
  parameter or builtin variable just before it (`Declaration::shadows`, an id into
  `Session::shadow_names`); a name that only a slot left by an earlier iteration or bar holds is not
  an outer binding. Only a flagged declaration saves the outer value, onto `Session::shadow_stack`,
  after its initializer ran (so a `:=` in the initializer is the outer variable's and stays) and
  before the local is set; `run_block` puts back down to its mark, in reverse, however the block
  ends (each branch, arm and loop iteration is its own execution). A `:=` above the declaration or
  in another branch stays too. A loop's counters work the same way (`ForRange::shadows`,
  `ForIn::shadows` for the index and the value): a counter that hides a visible name saves the outer
  value after the bounds or the collection were evaluated and puts it back when the loop ends; one
  that hides nothing stays in the table, so `i[1]` reads the previous bar's last count. Only
  `var`/`varip` locals are set at entry (stash value, else unset so the
  declaration initialises them), so one hides a global of its name for the whole call. Scopes are
  lexical: a nested call first sets each name its calling function's locals hide back to the global
  (`uncover_caller_globals`, from the caller's `saved` on `Session::call_frames`; by induction each
  call starts on the globals) and gives the caller its values back after
  (`cover_caller_globals`). Only locals named like a script global (top-level declarations,
  `scopes::top_level_declarations`; first in `RuntimeFunction::saved_order`, `global_locals` of
  them) are looked at:
  a call from the script's top level, or from a function whose locals share no global's name,
  costs nothing. The debugger's frame gets the caller's values first, then `saved`.
  `call_frames` is kept inside `request.security`: a function there reads the chart's global.
- `varip` (`Session::intrabar`): globals kept by name, user-function locals by call-site stash key
  (`invoke_with`'s `stash_key`, via `intrabar_key`). After an unconfirmed tick
  `capture_intrabar_objects` keeps what each `varip` variable and `varip` UDT field points at plus
  every reachable array, map, matrix and object (they live in the rolled-back `RuntimeState`;
  imported types' `varip` fields indexed too); the bar's next tick gets them back. `intrabar`
  cleared at a new bar and at the close. Never add `RuntimeState` fields for this: cloned per tick,
  one extra set measured 11% slower with four sessions.
- Builtin variables read with `[]` (`hlcc4[1]`, `ta.tr[1]`) are recorded by `read_history_builtins`
  at each confirmed bar's end; names found once by `history_builtin_names`. Not inside
  `request.security`.
- Breakpoints and watches: local `scriptDebug.state` setting (never synced; in `sync::record::policy`'s local
  keys).
- `once` finished flag: `RuntimeState::once_finished` (by site key), deliberately in working state:
  a rolled-back unconfirmed realtime tick keeps the structure active until the block runs on a
  closing tick (as TradingView documents). Condition evaluated every bar (like `if`) so `ta.*` keeps
  history.
- `matrix.*`: `matrix_builtins` (names in `MEMBERS`, offered directly by the editor).
  - Element arithmetic (`sum`, `diff`, `mult`) via `operators::apply` (ints stay ints, `na`
    propagates); float numerics (`pow`, `kron`, linear algebra) convert to
    `pine::builtins::matrix::linalg::Dense` (NaN = `na`) and back.
  - Bad shape/operand `PINE4037`; index/range outside `PINE4010`.
  - `Bound` holds six arguments (`matrix.fill`); more needs `BOUND_CAPACITY` raised and
    `tools/pine-bench.sh` run.
  - `det`, `inv`, `pinv`, `rank`, `eigenvalues`, `eigenvectors` in `linalg::Dense`: LU for
    `det`/`inv`, one-sided Jacobi SVD for `rank`/`pinv`, EISPACK `tred2`/`tql2` for eigen. `na` cell
    gives `na` (matrix/array of `na` for the others); singular `inv` is a matrix of `na`: data never
    stops a script. Eigen only for symmetric matrices, else `PINE4037` (TradingView names the same QL
    algorithm, silent on others).
- Drawing functions are exactly `drawings::{LINE,LABEL,BOX,LINEFILL,TABLE,POLYLINE}_MEMBERS` and
  `CHART_POINT_NAMES` (`handles` takes no `set_*`/`get_*`): misspelt members are compile errors;
  editor offers these names. Drawings store bar indexes and `time_anchored`; `set_xloc` re-reads
  coordinates and flips it. Label `yloc` stored with its price, ignored (may be `na`) above/below the
  bar; `PineLayer::label_y` resolves it against the candle, price scale unaffected.
- `table.cell_set_*` creates a blank cell if undefined (inferred). Numeric text size is
  `Size::Points(n)` (`font_size` scales TradingView's 14 points to our 12). Undrawable properties
  (font family, bold/italic) warned via `ignored_features`, never silently dropped.
- Hover tooltips (`tooltip=`, `label.set_tooltip`, `table.cell_set_tooltip`):
  `hover_targets_in_order` gives drawn rectangles in paint order across scripts (behind-chart labels,
  other labels, then all tables; filled bubble includes pointer) and must keep sharing
  `label_geometry` and `table_placements` with the painters. `pine_chart_render::target_at` picks the
  topmost. Only `LiveOverlays` looks (`Overlays::pointer` is `None` for snapshots), showing egui's
  tooltip on a copy of the canvas response over the target (egui's delay and "only when still"
  apply). Repeated `table.cell` resets it. Unverified vs TradingView: delay, wrap width (320 pt
  here), crosshair hiding.
- `plotarrow()` = `ArrowOutput` (per-bar value, both colours; `offset`/`show_last` applied by
  `ArrowOutput::finished` on read). Length `min_height`–`max_height` proportional to value vs the
  series' largest absolute value (inferred); up arrows under the low, down over the high.
  `plot.style_stepline_diamond` = `PlotStyle::SteplineDiamond` (diamond where value changes). Shapes
  in `pine_geometry` (`arrow_shape`, `diamond_points`); test there.
  `ignored_features::ignored_values` (PINE7004) is empty but kept for the next unsupported constant.
- `ta.pivot_point_levels`: running period OHLC in `ta::State::pivot` (`ta_extra::PivotPeriod`),
  starting at each anchor bar; levels set at a period's end hold until the next ends; `developing`
  uses the running period. Classic published formulas (reference lists none); Woodie's uses the
  setting bar's open. Unchecked against TradingView output. `ta.rci` = Spearman
  (Pearson on mid-ranks) x 100.
- `session.*` reads `trading_session::MarketSessions` by `syminfo.type`: crypto 24x7 UTC (session =
  UTC day); `stock` New York 09:30-16:00 weekdays, pre- from 04:00, post- to 20:00. Charts show the
  regular session, so `isfirstbar`/`islastbar` equal their `_regular` forms; a bar of a day or more
  is a whole session (`ismarket` true, pre/post false). First/last bar from the neighbouring bar's
  open time (`histories["time"]`), stateless. Known gap: `hour`, `dayofweek`, `time(..., session)` and
  other calendar parts default to UTC, not the exchange zone, so stocks differ from TradingView;
  `syminfo.timezone` and `session.*` use the exchange zone.
- `runtime.calculation.mode` = `Session::alert_mode` (set by `ExecutionController::with_alert_mode`
  for alert subscriptions). `timeframe.main_period` uses `Session::chart_bar_seconds`, set by the
  security evaluator while `bar_seconds` is the requested timeframe's. `weekofyear`: Sunday–Saturday,
  week 1 holds January 1 (inferred; reference only says "week number of the year").
- `strategy.*` statistics: `strategy_builtins::statistic`, computed from the broker per read (trade
  lists) except equity extremes, kept by `Broker` as bars arrive (`EquityExtremes`, rolled back with
  state) and merged with the running bar. Losses negative in averages (`avg_losing_trade`), positive
  in `grossloss`; drawdown from running peak (starting at initial capital), run-up from running
  trough; percentages against those. `margin_liquidation_price` stays a warned `na` (no margin model).
- Trades carry order comments (`OpenTrade::entry_comment`, `Trade::{entry,exit}_comment`; exit
  `comment_profit` / `comment_loss` replace `comment` on limit / stop fill), shown in trade-list hover
  (`Trade::comments`) and MCP, and highest/lowest seen price for
  `strategy.{closed,open}trades.max_runup/max_drawdown(_percent)` (positive, pre-commission, closed
  quantity). `Broker::process` touches open trades at the open, each trigger price and intrabar path
  vertex (a trade closed at the next open skips that bar's range); with no pending orders, high and
  low.
- `strategy.risk.*` sets rules on `Broker::risk` (latest wins; invalid call changes nothing).
  `Broker::process` rolls the trading day (exchange zone, `Broker::zone`); `check_risk` runs after
  each bar's equity is recorded. A hit limit cancels pending orders, closes trades at the bar's
  close, then `place` ignores orders: `max_drawdown` (from peak equity, cash or %) and
  `max_cons_loss_days` for good; `max_intraday_loss` (from the day's best equity) and
  `max_intraday_filled_orders` (per filled order, reversal counts once) until next day.
  `allow_entry_in` and `max_position_size` act on `strategy.entry` only. Checked at bar closes, so a
  limit can be passed intrabar unseen. OCA (`oca_name` with `oca_type` cancel or reduce) applies to
  entries and orders; an exit's own `oca_name` is still ignored.
- `input.*` arguments matched by name or position in `pine::inputs::arguments` (per-function order;
  `input.int`/`float` take a range, or `options` when the third place is a list).
  `InputDefinition::control` gives the editor beyond the value type: `input.price` is a float;
  `symbol`/`session`/`text_area` and `timeframe` without `options` are strings with own controls
  (`pine_studio::inputs`). Stored values stay plain `InputValue`, so saved inputs are unchanged.
  `display` is warned.
- `active`: compiled by `pine::inputs::activity` into `InputDefinition::active`, an expression over
  inputs declared above (by name) and script constants (folded to literals), evaluated by
  `InputSchema::is_active` on edited values with `compiler::constant_value` (also folds comparisons,
  `and`/`or`/`not`, `?:`). Failing → card disabled; input keeps its value, script reads it. Anything
  else (series, call, later input, non-bool) warns `PINE7001` and leaves it editable. MCP input tools
  ignore it.
- Declaring a builtin variable's name (`open = 1`, local `float hl2 = ...`, loop counter):
  `PINE7006` warning; `CE10190` if the builtin was read above it (`structure_validator::check_name`).
  Neither "always an error" nor the manual's "any read anywhere": the corpus accepts a read below a
  local shadow outside its block (`aVFtPnUh`) and unread shadows of `low`, `hl2`, `second`
  (`mwTRA4dO`, `aVFtPnUh`, `5YEHyvSL`). Parameters may reuse the name with a warning. TradingView's
  message text is undocumented.
- Keep execution linear in bars: continue historical bars in working state
  (`Session::working_is_committed`), move `request.security` site state in/out, share function bodies
  with `Arc`. Benchmark: `PINE_CORPUS_BARS=5000 PINE_CORPUS_REPORT=1`.

### request.security slices

A `request.security` expression that reads script variables (`longTrend` in
`request.security(sym, "15", longTrend)`) needs the statements that compute them run on the requested
series. `runtime::security_slice` finds them at build time, per request.

`syntax::scopes` (`resolve`, once per `build`) is the single scoping authority for static analysis:
it maps every identifier read, call head, declaration, assignment target, loop variable and function
parameter to a `Binding` (`Global`, `Local(LocalId)`, `Parameter { function, position }`, `Free`).
Keyed by source offsets, so it survives cloning the tree. An analysis asks it; it does not re-derive
Pine's scoping from names. Its rules are the ones `invoke_with` runs with (below), so a change to
how a name is scoped changes `scopes.rs` and its tests, not each analysis.

The module is a directory: `mod.rs` (`build`, `slice_for`, the position rule), `effects.rs` (what
functions and statements change), `reads.rs`, `locate.rs` (the blocks around a request), `members.rs`
(the builtin member tables).

- **What is sliced.** The statements **before the request** that declare, `:=`-reassign or may fill
  what the expression reads, through other statements and the script's functions. A statement may
  fill a collection through `array.push(samples, x)`, `samples.push(x)`, a call of the script's own
  function that changes the parameter it gets the variable as, or the global collection it changes
  (`Functions::{mutated, global_writes}`, a fixpoint over the call graph). A function that only reads
  its argument is not replayed. Unknown collection members count as mutating: `READ_ONLY_MEMBERS`
  lists the array/map/matrix queries and matrix predicates that do not.
- **Where the statements come from.** The top-level ones before the request's own, and those before
  the request in the blocks around it (`Location`, found by `locate`; replayed without re-checking
  the block's condition, since the request was reached). `evaluate_higher_timeframe` runs the slice
  in the request's own state, before the expression, with its own empty `Session::intrabar` (a
  `varip` there acts like `var`; the chart's is untouched).
- **Scoping, aliases and overloads.** Reads and writes are scope-aware, through the bindings: a
  block's own `level` is not the global. Plain-copy aliases are followed (`alias = samples`, also one
  a block or a helper makes, and one an inner block reassigns); a builtin's collection argument is
  found by its `id` label or its position. Aliases are per local (two blocks' `alias` are different
  `LocalId`s): `Walker` keeps what each was copied from as it walks in source order, a block's
  additions reach the enclosing block when the statement they belong to ends and never a sibling
  block, and the globals' aliases are groups of names (`Aliases`). Decisions that depend on the
  runtime's name-keyed variable table stay by name (a slice that would define a global a
  request-block local of the same name hides gets no slice). A function call follows the overload
  the type checker resolved (`Call::target`), all same-name ones when unresolved; a function's
  parameters, and the names its body declared before reading them, are not
  reads of a global (a `Local` or `Parameter` binding), the same scoping `invoke_with` runs them
  with. A dotted call of a builtin namespace (`ta.sma`) never matches a script function called
  `sma`, for writes as for reads, and a call the type checker resolved to a script method has that
  method's own effects, not the builtin ones of its name.
- **Position rule.** A name that is rebound (declared, `:=`) counts only before the statement that
  reads it (`Facts::writes_needed`, a limit per needed name). A statement's `Facts` are computed once
  (`FactsCache`), however many requests it precedes. A collection changed through its value,
  a `var`/`varip` and a `level[1]` read also see later writes.
- **No slice.** A statement in a block cannot be replayed when it declares something, reads a name a
  block declared, or sits in a loop or `once` body: when it would be part of the slice, the request
  gets no slice and reads the chart's values. A name that a block around the request declares before
  it (`level = 20.0` in the `if` it is in, a loop variable) is not resolved to the global of that
  name: the expression reads the chart's value (`security_globals`), and when the slice would define
  a global of that name the request gets no slice. A statement that makes a `request.*` call is never
  in a slice (nesting; its names keep the chart's value via `security_globals`), and requests inside
  functions are not sliced.
- **Alias groups.** Only script globals (`Binding::Global` on both sides of a copy) alias, and a
  copy (`a := b`, `a = b`, also in the blocks of an expression) reaches only the changes that start
  after it in the source, or any when it copies into a `var`/`varip`, which keeps it for the next
  bar (`Aliases::reaching`). A copy after a change in the same loop body is not seen as reaching it.
- **Alerts.** `alert()` does nothing while a request runs (`security_depth > 0`: a slice, or a
  function the expression calls): only the chart's own run fires alerts, and the once-per-bar ledger
  stays its own.
- **Adding a read-only member.** Add a new array/map/matrix query to `READ_ONLY_MEMBERS`, or a
  slice will treat it as a write and replay more than needed.

## Editor

- Editing assistance: pure `pine_studio::editor::edit_ops`, not `editor.rs`; text + selection in,
  one `Edit` or `None` out. Character-offset indices; four-space indent; Pine braces unpaired.
- `pine_studio::editor::syntax` is the sole per-text classification: strings from the lexer, comment
  scanning respects strings, names match whole tokens. Never use lexer indent/dedent for editor
  indentation (incomplete parentheses erase them). Outputs `SyntaxRole`s, never colours;
  `classify(source, Some(&table))` takes cached `EditorState::symbols()` (scope-aware, authoritative
  for names), `classify(source, None)` is the lexical fallback for snippets outside the editor.
  - Colours: `ui::editor_appearance` (`current()` editor, `chrome()` code on app background); never
    `theme::syntax` or hard-coded colours in editor code. New role/surface → `model/editor_theme/roles.rs`
    and every built-in theme.
  - Font: `editor_appearance::editor_font()` (size, line height, `editor-code` family, default
    built-in JetBrains Mono: `theme::bundled_editor_font`, OFL, see `assets/fonts/README.md`); layout
    constants follow it (`gutter_width`, `chrome_scale`). No 13-point assumption.
  - Theme and font owned by `host::editor_theme` (saved after a pause, never while Settings
    draws); change through it so all windows repaint. See [Editor themes](../editor-themes.md).
- Completion never compiles, runs, re-lexes or touches network/storage. Builtins from the symbol
  catalog, signatures from `pine_studio::editor::signature`: add new builtins to both. `array.*`/`map.*`
  come from `array_builtins::KNOWN` / `map_builtins::NAMES` (signatures `parameters()`), no editor
  edit needed. List shows `COMPLETION_ROWS` (10) and scrolls to the highlight on ↑/↓;
  `completion_keys` consumes keys before drawing, so set `CompletionView::scroll_to_selected`, don't
  read `key_pressed`. Accepting an `Item` with `callable` (builtin functions, user `name(args) =>`)
  writes `name()` with caret inside, unless `(` follows.
- `pine_studio::editor::diff` is the sole saved-vs-source comparison. Cache `EditorState::changes()`
  for gutter, status, popover, revert; never diff while painting. Hunks: zero-based half-open ranges,
  terminators preserved; deleted hunks at `new_lines.start`, no fake rows. `is_dirty()` stays a string
  comparison.
- `revert_edit` rejects stale text; apply via `apply_hunk_edit` (one undo step). Only
  `mark_saved`/`replace_source(saved)` move the baseline. Colors: `theme::editor_diff_*`.
- `pine_studio::editor::symbols` (go-to-definition, rename) reads lexer tokens, not the AST (no spans
  for parameters, loop variables, tuple names, aliases; drops unparsable statements), so it works
  mid-edit. Scopes from each statement's raw leading whitespace; declarations visible from the end of
  their statement and block; values and callables separate namespaces; members, fields, `lib.fn`
  unresolved. Cache `EditorState::symbols()` by text hash. Renames are never reasoned about:
  `SymbolTable::rename` applies, re-analyses, refuses unless every name resolves to the same
  declaration. Test with CRLF and astral characters; ranges are char indexes.
- Multiple carets: egui owns the *primary* caret; `editor/carets.rs` keeps others in
  `CaretState::extra`. With extras, `carets::apply` takes editing/moving keys before egui into pure
  `pine_studio::editor::multi_cursor` (`edit_ops` per caret, one shared scan, edits applied together;
  overlap drops the later caret). Any other text change (egui typing, undo, find/replace, rename,
  sync) fails the `hash` check and drops extras (so undo collapses them); never give extras their own
  undo history. Escape blurs egui's field before editor code, so `apply` reads
  `had_focus_last_frame` and re-takes focus. Clicks: `note_pointer` before egui moves the caret,
  `resolve` on laid-out text after; ⌘/Ctrl-click jumps on a defined name, else adds a caret; ⌥-click
  always adds one.
- Suggestion list and signature panel: `egui::Area`s on their own layer (outside the editor's clip
  rect) following `caret_rect`. `EditorState::caret_hidden` (caret outside visible text, set in
  `show`) hides both and drops the list so Enter/Tab cannot accept an invisible row. Never
  `Area::constrain_to` (pins to the edge instead of hiding).
- Jump flash: `Flash`, the landed character or (`show_line`: profile hot lines, debugger frames) a
  whole-line band behind text that holds, then fades over 700ms.
- Constant colours get a swatch (`editor/swatches.rs`, [Pine colour picker](../pine-color-picker.md)),
  its room a section's `leading_space` from `syntax::layout_job_with_gaps`: text and offsets
  untouched, but such rows are wider than `columns * char_width`; take positions from the galley,
  never a column. Detection: lexer tokens in `pine_studio::editor::color_sites`. Picker edits rewrite
  from the gesture's starting text, one undo step via `UndoGesture`, never a second undo stack.
- Editor rail: `pine_studio/rail.rs` mounts a right panel around the whole editor column (code,
  assistant, debugger) and holds the assistant, profiler and debugger buttons, the preview chart's
  position menu and toggle. It is drawn by `kit::bar::show_column` (the bar's buttons, vertical,
  icon only); add a panel's button there, not to the top bar, whose right run is empty.
- Right-click menu, ⌘-hover cue, inline rename box: `editor/navigation.rs`. An egui `Area` takes no
  focus on its first frame (only measures), so the rename field asks on the second.
- Menu strip: `pine_studio/menu_bar/` draws File, Edit, View and Theme at the top of the editor
  column (a `Panel::top` inside `rail::show`'s remaining area, so it spans the code and the assistant
  and debugger panels). Rows are plain `Entry` data built by `file`, `edit`, `view` and `themes`,
  drawn and sized by `entry` (`fitted_menu_rows`, submenus `menu_fit_rows` inside; an `Entry::Scroll`
  block scrolls past `kit::MENU_MAX_HEIGHT`); a row's command is a `Cmd`. The problems chip
  (`editor::status::ProblemsChip`) sits at the strip's right end, not under the code.
  - **Theme menu** (`themes.rs`): Automatic, the built-in themes and the person's own, as Settings'
    tiles list them, the one worn checked, then "Configure Themes…". The studio holds the app's
    `host::editor_theme::EditorThemeHost` (`PineStudio::new` takes it; `with_script_store`, for tests,
    a placeholder that publishes nothing). The rows come from `EditorThemeHost::choices()` (the worn
    theme and the custom names, no colours), read only while the menu is up. `Cmd::SetEditorTheme`
    runs `EditorThemeHost::select`, the same call as a Settings tile, so every window repaints;
    `Cmd::OpenThemeSettings` pushes `PineStudioAction::OpenEditorThemeSettings`, which the app turns
    into `SettingsHost::open_editor_themes` (the Pine Studio page).
  - **Every item of every context menu in `pine_studio/` is in the strip**, except the rows of a
    list that act on that row alone (Problems, Logs, the debug panel's variables, watches and
    breakpoint list). A context menu and the strip build their rows from one function so they cannot
    drift: `library::script_actions` (picker row menu and File), `editor::breakpoint_rows` and
    `change_rows` (the gutters' menus and Edit's Breakpoint and Change submenus). A new item goes
    into that function, not into a copy.
  - **Edit-menu actions** are `editor::EditAction` (`edit_actions.rs`), requested with
    `EditorState::request_edit` (`Cmd::Edit` from the strip, the app from the macOS menu) and run at
    the start of the editor's next frame. Undo and Redo step egui's own `TextEditState` undoer the
    way ⌘Z does (feed the text, then `undo`/`redo`): there is never a second history. Toggle
    Comment, Duplicate and Move Line call `assists::line_edit`, the function the keys use.
  - **`EditorState::menu_state`** is the Edit menu's enablement. It reads the *stored* caret and
    selection, never the keyboard focus (the native menu's `update_menu` is gated on focus and is
    left alone). Called only while the Edit menu is up: undo availability clones the history.
    What costs a scan of the text (declaration, usages, whether a line can move) is cached by text
    hash, selection and caret.
  - **Keyboard.** A click on the strip makes the editor surrender focus, and an unfocused
    `TextEdit` collapses its selection, which would grey out Cut and Copy as the menu opens. So the
    strip requests focus back for the editor each frame a menu is up (if the editor had it when the
    menu opened) and the frame after a row was chosen, but only while nothing else holds the
    keyboard: the find field, the picker's search or a sheet's field that took it keeps it. The
    strip (and the picker) consume Esc after they have used it, so the editor under them does not
    also close its find bar or drop its extra carets.

## Drawer tabs

- Report, Problems, Profile share `pine_studio::drawer::tab_chrome`: 46px toolbar with a
  `state_field`, hairline, cards of rows. Logs (`ui::pine::logs_view`, shared with the chart
  workspace) mirrors it; keep `TOOLBAR_HEIGHT`, `MARGIN`, `BUTTON` equal.
- Width before height: `chrome::wide` (520pt) puts parts in `chrome::columns`;
  `pine_logs_view::intro` puts a picture beside its steps. Nothing outside a scroll area may exceed
  the drawer (minimum 90pt): bodies `chrome::scroll`, empty states `centered` (scrolls), scroll areas
  `min_scrolled_height(0.0)` (egui's default 64 overflows a short drawer). `drawer_tabs_tests` asserts
  fit at five sizes; `DEGENVIEW_GALLERY_DIR` for pictures.
- Empty tabs: `centered` + `pine_logs_view::intro` (picture, three steps, example; see
  `drawer_intro`), never a bare text placeholder. Inputs too (`drawer_intro::inputs`,
  `drawer_tabs_tests::inputs_fit_the_drawer` at five sizes).
- Pine Studio with no scripts (`pine_studio::library::empty_library`): detail area `centered` +
  `intro`, the two ways to begin as footer; the script picker's list a short form with the same
  picture (`empty_library::short`). No-match search stays the small "No matches" tile. Pictures: `cargo test --lib empty_library -- --ignored`
  with `DEGENVIEW_GALLERY_DIR` set.
- Problems copy button copies all problems (errors first); right-click a row copies it or just its
  message.
- `pine_report::show` / `report_content` (chart settings) keep the compact look. The drawer's Report
  has its own: `drawer/figures.rs` (net profit large, key figures, quieter ones, aligned `grid`),
  `drawer/plot.rs` (the equity and simulation charts: scale at the right edge with the capital,
  readout under the pointer), `drawer/trades.rs` (aligned columns; bars held gives way first when
  narrow). Pictures in both themes: `cargo test --lib gallery_of_the_report -- --ignored` with
  `DEGENVIEW_GALLERY_DIR`.
- Report's Simulation card (`drawer/simulation.rs`) shows `pine::strategy::monte_carlo`: closed
  trades redrawn with replacement (fixed seed, so a report always gives the same figures). Drawing
  with replacement is what spreads the final equity (a sum of the drawn profits, so order never
  changes it); order only shapes the path, so it matters to drawdown. Say that, never "how much came
  from the order". Profits are added as money, so a percent-of-equity strategy is approximated, not
  compounded. Takes `MIN_TRADES` (10) to `MAX_TRADES` (20,000) closed trades: runs shrink from 1,000
  to 100 as the list grows, and `DRAW_BUDGET` is never exceeded, which is why a longer list is
  refused (`Unavailable`) instead of run. A loss is judged on the run's accumulated gain with a
  rounding tolerance, not `equity < initial`, and drawdown is not capped at 100%. Frames reuse the
  answer (or the refusal) until `trades_fingerprint` changes; never run the resampling per frame.
  Figures are painted galleys, so each carries an accessibility label (`Figure::spoken`). Compact
  chart settings report does not show it.

## Manager lifecycle

- **Scripts edited outside the app** (a person's editor, a coding agent in the Scripts folder) reach
  the open editor via `pine_studio::library::external` (pure state machine; `mod.rs` does I/O,
  `disk_banner` asks). The library rescans every 1.5 s (`PineStudio::begin_frame`); the open script's metadata is read every
  ~400 ms (`ScriptStore::file_stamp`); a moved or settling file triggers a rescan and
  `reconcile_open`. Act only on the same text seen twice, 300 ms apart (never load half-written
  files).
  - **Clean editor**: show file text via `editor::apply_script` (one undo step) + `mark_saved`;
    "Reloaded" note in the status strip.
  - **Unsaved typing is never overwritten**: banner offers *Reload from disk* (undoable) or *Keep my
    version* (`rebase_saved`: typing stays, file text becomes the saved baseline, so a later Save
    overwrites on purpose); Save held while open.
  - **Saving checked at the store**: `save_open` uses `ScriptStore::save_if_unchanged(id, source,
    saved_source)`; a file rewritten since the last look is never silently overwritten (next look
    asks at once, `Watch::force`). Equality ignores a missing final newline (`external::same_text`).
  - Not covered: chart instances keep their added source (as after any Save); outside edits make no
    revision; drafts of non-open scripts are unprotected from outside edits.
- `PineStudio` owns its window. `workspace::PreviewState` drives one crypto/stock preview (debounced
  apply, stale output, no alert coordinator). `pine_studio::inputs` renders a precompiled
  `InputSchema`, shared with chart settings.
- Scripts can fail at run time after compiling (some names judged only when run).
  `pine_studio::drawer::problems` is the one problem list: compile diagnostics plus chart and profile
  failures, each only if its run was of the editor's current text, never beside a compile error.
  Problems tab, badge, editor chip and red underline all read it; never handle a failure in one place
  only. `ExecutionController` keeps `last_error`; a no-change update repeats it (else it reads as
  success).
- **New builtins are registered in `pine::compiler::registry`**, which gathers module name lists
  (`array_builtins::KNOWN`, `math::SUPPORTED`, `drawings::NAMESPACES`, `strategy_builtins::NAMES`, …)
  plus inline-evaluated names (`TA_INLINE`, `STR_INLINE`, `OTHER_FUNCTIONS`); compiler and completion
  read it. Module tables remain the interpreter's dispatch: add there **and**, if inline, to the
  registry. Two guards: every registered function runs and is not `PINE4007` (registry ⊆ runtime);
  every `"ns.member"` literal under `src/pine` in a closed namespace is registered or a known value
  (runtime ⊆ registry).
- **Closed** namespace (`registry::CLOSED`): unregistered call is `PINE3053` error with "Did you
  mean". **Value-closed** (`CLOSED_VALUES`): unknown variable read is `PINE3050`. Add a namespace only
  when both guards pass and the Pine corpus is unchanged. `request` stays open (own `PINE9003`);
  `syminfo`, `strategy`, `runtime` stay open for values (Pine adds variables; `syminfo.cftc_code`
  failed the corpus). A false error on a working script is worse than a missed typo.
- **Own functions and methods are bound too.** The type checker resolves a callable (`callables.rs`;
  id = position among top-level functions); `call_binding::bind` writes `Expression::Call::target`;
  `Session::call_target` runs `Session::callables[target]`, a dotted call's receiver as first
  parameter (`Supplied`). Untypeable calls keep the by-name path in `user_methods.rs`.
  `Session::ignore_routes` disables both `route` and `target`. Read
  [User-defined methods](../pine-user-methods.md) before changing either side; check with
  `tools/pine-bench.sh` (`user_functions*` / `user_methods*` twins must stay equal).
- **Builtin calls are routed, not searched** (a per-execution search of `Session::call`'s families
  cost ~100 string tests before `ta.sma`, 180 before `plot`). At session build `call_binding::bind`
  sets `Expression::Call::route` on every call nothing in the program can capture (no function,
  method, type or import alias of that name or head); `call_routed` (`call_routes.rs`) goes straight
  to the family. The chain handles the rest (user functions/methods, variable receivers like
  `levels.push`, `request.*`, `log.*`, `ticker.*`, debugger watch expressions, parsed separately and
  never bound) and is the backstop raising `PINE4007`. Each family is one method both entrances call.
  - **New builtin → a family. New family → a `Route`, an arm in `route_of` in chain order, an arm in
    `call_routed`.**
  - Fail-safe: a family returning `None` hands back to the chain (wrong route costs time, not
    correctness).
  - Guards: `call_binding_tests` (every registered function both ways, four argument shapes;
    workloads both ways; user methods/types/aliases that must stay on the chain) and
    `PINE_CORPUS_DUAL=1` (every community script both ways). Bench with `tools/pine-bench.sh`:
    `many_ta`, `many_math`, `many_str`, `plots_and_drawings`, `containers`, `map_counting`.
- A run-time failure has no profile (`Profiler::finish`), only the failure, also in Problems.
- Never close a Pine Studio tab or quit over unsaved editor text or retained drafts without the
  `unsaved` flow. Tab close starts `begin_exit(CloseTab)` and cancels close; quit/update restart use
  `DegenViewApp::request_quit` plus the native termination intercept. Failed save cancels exit.
  Don't Save restores saved text and calls `ScriptStore::discard_draft`.
- `pine_studio::library::demo` seeds examples once and on demand from an empty state; deleted demos
  stay deleted. Names = filenames, no “Demo” prefix. Examples: original GPL-compatible work, zero
  diagnostics including warnings.

`src/pine/tests/corpus.rs` uses the optional cached community corpus in `tools/pine-corpus/`. Reduce
failures with original code; never copy corpus scripts into the repository.

## iCloud Sync

User scripts sync as files via `sync::record::scripts::ScriptSink` (`host::sync::scripts::StoreScriptSink` over `store::scripts::ScriptStore`).
Drafts and revisions never sync; unmodified bundled examples are neither uploaded nor kept over an
incoming same-name script; text being typed is never overwritten (incoming waits in the sync inbox).
Two edits to one script keep both; the loser becomes a conflict-copy script. Sync code writes script
files only through the sink. See [iCloud Sync](../icloud-sync.md).
