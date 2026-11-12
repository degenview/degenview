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
  Check `session.rs` changes with `tools/pine-bench.sh <base-rev>` (interleaved A/B; checksums prove
  both builds ran the same program). Watches/conditions must parse with
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
    run; a new chart-run input (`run_revision`, `ChartCard::pine_outputs`) goes into the identity and
    `DebugFeed`.
  - Bench: `tools/pine-bench.sh <base-rev> <head-rev>` (head must be a commit; `profiled_*`
    workloads = recorder cost).
- `request.footprint()` handles are stateless (footprint = bar index; `volume_row` = bar index +
  row); data in `Session::footprints`, outside `RuntimeState` so rollback does not copy it. Read
  [Pine volume footprints](../pine-footprint.md) before changing the algorithm: mostly inferred.
- Execution-model gaps: [the audit](../reviews/pine-execution-model-audit.md); read it before relying
  on rollback exceptions. Still open: broker orders do not survive a rolled-back tick; runtime errors
  do not halt the script.
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
- Right-click menu, ⌘-hover cue, inline rename box: `editor/navigation.rs`. An egui `Area` takes no
  focus on its first frame (only measures), so the rename field asks on the second.

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
  `intro`, the two ways to begin as footer; sidebar list a short form with the same picture. No-match
  search stays the small "No matches" tile. Pictures: `cargo test --lib empty_library -- --ignored`
  with `DEGENVIEW_GALLERY_DIR` set.
- Problems copy button copies all problems (errors first); right-click a row copies it or just its
  message.
- `pine_report::show` / `report_content` (chart settings) keep the compact look; the drawer lays out
  `metric_grid`, `equity_curve` itself.

## Manager lifecycle

- **Scripts edited outside the app** (a person's editor, a coding agent in the Scripts folder) reach
  the open editor via `pine_studio::library::external` (pure state machine; `mod.rs` does I/O,
  `disk_banner` asks). Sidebar rescans every 1.5 s; the open script's metadata is read every
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

User scripts sync as files via `sync::record::scripts::ScriptSink` (`StoreScriptSink` over `ScriptStore`).
Drafts and revisions never sync; unmodified bundled examples are neither uploaded nor kept over an
incoming same-name script; text being typed is never overwritten (incoming waits in the sync inbox).
Two edits to one script keep both; the loser becomes a conflict-copy script. Sync code writes script
files only through the sink. See [iCloud Sync](../icloud-sync.md).
