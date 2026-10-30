# Pine and Script Manager guidance

## Runtime

- Everything for the Pine engine stays under `src/pine/`; it and Pine domain types in `model/` are
  pure and contain no egui types. Alerts and subscriptions compile the same modules.
- Unsupported behavior is never silent. Stand-ins emit `ignored_features` warnings
  (`PINE7001`–`PINE7006`) and live in `misc_builtins`; object results or feeds without a stand-in
  remain `PINE9003` errors.
- `log.*()` messages go to `Session.logs`, never into `RuntimeState` (rollback would discard them).
  Capture is opt-in (`ExecutionController::with_logs`); a reader applies `LogBatch`es in order
  through `LogHistory`. Keep the limits in `pine::logs` and read [Pine Logs](../pine-logs.md) before
  changing them.
- The debugger (`pine/debug/`, [Pine debugger](../pine-debugger.md)) is opt-in: `Session::debug` is
  `None` unless `attach_debugger` ran, and the hooks (statement loop, `invoke`, `execute_bar`, the
  `request.security` evaluator) test it and call `#[cold] #[inline(never)]` helpers. Do not add a
  hook to `eval`, `budget` or any per-expression path, and do not add work to the statement loop that
  runs without a debugger. Check a change to `session.rs` with `tools/pine-bench.sh <base-rev>`
  (interleaved A/B, checksums prove both builds ran the same program). A watch or condition may
  only be an expression `pine::debug::parse_expression` accepts: add to its allow-list only a call
  that neither writes state nor allocates a handle, and test it against a clean run.
- The profiler (`pine/profile/`, [Pine profiler](../pine-profiler.md)) shares the debugger's gate:
  `Session::hooks` is `None` unless `attach_debugger` or `attach_profiler` ran (one or the other), the
  statement loop, `invoke` and `execute` test it and call `#[cold] #[inline(never)]` helpers. Keep the
  statement loop's shape: an earlier version with a block-level test and a second loop measured 1–2%
  slower in layout alone. Every `hook_before` needs its `hook_after`/`hook_failed`, every
  `hook_enter_function` its `hook_leave_function`. A region is a statement or a user function,
  identified by the offset the parser anchored it at; library code (`context.scope`) is never timed.
  A change to timing must keep `Σ line time + not attributed = total` exactly (the tests run on a
  stepping clock and assert it). Check a change to `session.rs` with `tools/pine-bench.sh <base-rev>
  <head-rev>` (head must be a commit; the `profiled_*` workloads are the recorder's cost).
- `request.footprint()` handles are stateless (a footprint is a bar index, a `volume_row` a bar index
  plus a row number) and the data lives in `Session::footprints`, outside `RuntimeState`; keep it
  there so rollback does not copy it. Read [Pine volume footprints](../pine-footprint.md) before
  changing its algorithm: most of it is inferred.
- Breakpoints and watches live in the local `scriptDebug.state` setting (never synced; listed in
  `sync::policy`'s local keys).
- A `once` structure's finished flag is `RuntimeState::once_finished` (by site key), in the working
  state on purpose: an unconfirmed realtime tick that ran the block is rolled back, so the structure
  stays active until the block runs on a closing tick, as TradingView documents. Its condition is
  evaluated every bar (like an `if`'s) so `ta.*` calls in it keep their history.
- `matrix.*` lives in `matrix_builtins` (names in `MEMBERS`, which the editor offers directly). Element
  arithmetic (`sum`, `diff`, `mult`) goes through `operators::apply` so ints stay ints and `na`
  propagates like the operators; anything numerical that returns floats (`pow`, `kron`, linear algebra)
  converts to `pine::linalg::Dense` (NaN stands for `na`) and back. A shape or operand a function cannot
  use is `PINE4037`; an index or range outside the matrix is `PINE4010`. `Bound` holds six arguments
  (`matrix.fill`), so a new builtin with more needs `BOUND_CAPACITY` raised and `tools/pine-bench.sh` run.
  Linear algebra (`det`, `inv`, `pinv`, `rank`, `eigenvalues`, `eigenvectors`) is `linalg::Dense`: LU for
  `det`/`inv`, one-sided Jacobi SVD for `rank`/`pinv`, EISPACK `tred2`/`tql2` for the eigen-decomposition.
  A `na` cell gives `na` (a matrix or array of `na` for the others) and a singular `inv` is a matrix of
  `na`, so a script is never stopped by data. The eigen-decomposition only exists for a symmetric matrix;
  anything else is `PINE4037` (TradingView names the same QL algorithm and does not say what it does
  for other matrices).
- Drawing functions are the exact lists `drawings::{LINE,LABEL,BOX,LINEFILL,TABLE,POLYLINE}_MEMBERS` and
  `CHART_POINT_NAMES`: `handles` no longer takes any `set_*`/`get_*`, so a misspelt member is a compile
  error and the editor offers each name from these tables. A drawing stores bar indexes and remembers
  whether it was placed by time (`time_anchored`); `set_xloc` re-reads the new coordinates and flips the
  flag. A label's `yloc` is stored with its price, which is ignored (and may be `na`) above or below the
  bar; `PineLayer::label_y` resolves it against the candle, so the price scale is not affected.
- `table.cell_set_*` creates a blank cell when the cell was never defined (inferred; the reference does not
  say). A text size is `Size::Points(n)` when a script gives a number (`font_size` scales TradingView's 14
  points to this app's 12). Properties the chart cannot draw (cell tooltips, font family, bold/italic) are
  accepted and warned about through `ignored_features`, never silently dropped.
- `plotarrow()` is `ArrowOutput` (per-bar value and both colours; `offset`/`show_last` applied by
  `ArrowOutput::finished` when the output is read, like a plot's offset). The arrow length is `min_height` to
  `max_height` in proportion to the value against the series' largest absolute value (inferred: the reference
  only says larger values draw longer arrows), up arrows sit under the bar's low and down arrows over its
  high. `plot.style_stepline_diamond` is `PlotStyle::SteplineDiamond`: a step line with a diamond where the
  value changes. Shapes are in `pine_geometry` (`arrow_shape`, `diamond_points`), which is where to test them.
  `ignored_features::ignored_values` (PINE7004) has no entry today but stays for the next unsupported constant.
- `ta.pivot_point_levels` keeps the running period's OHLC in `ta::State::pivot` (`ta_extra::PivotPeriod`): a
  period starts at each anchor bar, the levels set when one ends hold until the next ends, and
  `developing` recomputes from the running period instead. The type formulas are the classic published
  ones (the reference lists none) and Woodie's uses the open of the bar that sets the levels, so none of it
  is checked against TradingView's output. `ta.rci` is Spearman's coefficient (Pearson on mid-ranks) x 100.
- `session.*` reads `trading_session::MarketSessions` for the symbol's `syminfo.type`: crypto is 24x7 on UTC (a
  session is a UTC day), a `stock` keeps New York's 09:30-16:00 weekdays with pre- (04:00) and post-market
  (to 20:00) around it. The app's charts show the regular session, so `isfirstbar`/`islastbar` equal their
  `_regular` forms, and a bar of a day or more is a whole session (`ismarket` true, pre/post false). The
  first/last bar is worked out from the neighbouring bar's open time (`histories["time"]`), so it needs no
  state. Known gap: `hour`, `dayofweek`, `time(..., session)` and the other calendar parts still default to UTC
  where TradingView uses the exchange zone, so on a stock they differ from TradingView's; `syminfo.timezone`
  and `session.*` use the exchange zone.
- `runtime.calculation.mode` is `Session::alert_mode`, set by `ExecutionController::with_alert_mode` for the
  alert subscriptions. `timeframe.main_period` uses `Session::chart_bar_seconds`, which the security
  evaluator sets while `bar_seconds` is a requested timeframe's. `weekofyear` weeks run Sunday to Saturday and
  week 1 holds January 1 (inferred; the reference only says "week number of the year").
- `strategy.*` statistics are `strategy_builtins::statistic`, computed from the broker on each read (the
  trade lists) except the equity extremes, which `Broker` keeps as bars arrive (`EquityExtremes`, rolled back
  with the rest of the state) and merges with the bar being run. Losses are negative in the averages
  (`avg_losing_trade`) but positive in `grossloss`; a drawdown is measured from the running peak (starting at
  the initial capital) and a run-up from the running trough; the percentages are against that peak / trough.
  `margin_liquidation_price` stays a warned `na`: there is no margin model.
- A trade carries its orders' comments (`OpenTrade::entry_comment`, `Trade::{entry,exit}_comment`; an exit's
  `comment_profit` / `comment_loss` replace `comment` when its limit / stop fills it) and the highest and
  lowest price it has seen, from which `strategy.{closed,open}trades.max_runup/max_drawdown(_percent)` follow
  (positive amounts, before commission, sized to the quantity that closed). `Broker::process` touches the
  open trades at the bar's open, at every price a trigger fires at and at each vertex of the intrabar path, so
  a trade closed at the next open does not count that bar's range; a bar with no orders pending touches its
  high and low. Comments show in the trade list's hover text (`Trade::comments`) and over MCP.
- `strategy.risk.*` calls state rules on `Broker::risk` (the latest call wins; an invalid call changes nothing).
  `Broker::process` rolls the trading day (in the exchange zone, `Broker::zone`) and `check_risk`, called after
  each bar's equity is recorded, compares it with the limits. A reached limit cancels every pending order and
  closes every open trade at the bar's close, then `place` ignores orders: `max_drawdown` (from the peak
  equity, cash or % of it) and `max_cons_loss_days` stop for good, `max_intraday_loss` (from the day's best
  equity) and `max_intraday_filled_orders` (one per filled order, a reversal counting once) until the next
  day. `allow_entry_in` and `max_position_size` act on `strategy.entry` only. Rules are checked at bars'
  closes, so a limit can be passed within a bar before it is seen. OCA groups (`oca_name` with `oca_type`
  cancel or reduce) apply to entries and orders; an exit's own `oca_name` is still ignored.
- An `input.*` call's arguments are matched to parameters by name or by position in
  `pine::input_arguments` (each function has its own order; `input.int`/`float` take a range or, when the
  third place holds a list, `options`). `InputDefinition::control` says what edits it beyond its value type:
  `input.price` is a float, `symbol`/`session`/`text_area` and a `timeframe` without `options` are strings
  with their own controls (`script_manager::inputs`), and the stored value of each is the plain
  `InputValue` it always was, so saved inputs are unchanged. `display` is warned about.
- `active` is compiled by `pine::input_activity` into `InputDefinition::active`: an expression over inputs
  declared above it (kept by name) and the script's constants (folded to literals), evaluated by
  `InputSchema::is_active` against the values being edited with `compiler::constant_value`, which also
  folds comparisons, `and`/`or`/`not` and `?:`. The form draws a card whose condition fails disabled; the
  input keeps its value and the script reads it. Anything else (a series, a call, a later input, a
  non-bool) is warned (`PINE7001`) and leaves the input editable. The MCP input tools do not honour it.
- Declaring a name a builtin variable has (`open = 1`, a local `float hl2 = ...`, a loop counter) is the
  `PINE7006` warning unless the script read the builtin above the declaration, then `CE10190`
  (`structure_validator::check_name`). The manual calls it a warning that becomes an error once the script uses
  the builtin "anywhere"; the corpus shows that a read below a local shadow, outside its block, is accepted
  (`aVFtPnUh`), and that scripts shadow `low`, `hl2` and `second` with no read of the builtin
  (`mwTRA4dO`, `aVFtPnUh`, `5YEHyvSL`), so the rule is not "always an error" nor "any read anywhere". A
  parameter may reuse the name inside its function with a warning. TradingView's own message text is not
  documented.
- Keep execution linear in bars: continue historical bars in working state
  (`Session::working_is_committed`), move `request.security` site state in/out, and share function
  bodies with `Arc`. Benchmark with `PINE_CORPUS_BARS=5000 PINE_CORPUS_REPORT=1`.

## Editor

- Editing assistance belongs in pure `script_manager::edit_ops`, not `editor.rs`. Operations take
  text plus selection and return one `Edit` or `None`. Indices are character offsets. Indentation is
  four spaces; Pine braces are not paired.
- `script_manager::syntax` is the sole per-text classification. Strings come from the lexer;
  comment scanning respects strings; names match whole tokens. Do not use lexer indent/dedent to
  decide editor indentation because incomplete parentheses erase them. It produces `SyntaxRole`s,
  never colours: `classify(source, Some(&table))` takes the cached `EditorState::symbols()` (scope-aware,
  authoritative for names), `classify(source, None)` is the lexical fallback for snippets outside the
  editor. Colours come from `ui::editor_appearance` (`current()` for the editor, `chrome()` for code on
  the app's own background): never call `theme::syntax` or a hard-coded colour from editor code. A new
  role or surface goes in `model/editor_theme/roles.rs` and every built-in theme. The editor's font
  comes from `editor_appearance::editor_font()` (size, line height, the `editor-code` family) and its
  layout constants follow it (`gutter_width`, `chrome_scale`): do not add a 13-point assumption. The
  theme and font are kept by `host::editor_theme_host` (saved after a pause, never while Settings draws);
  change them through it so every window repaints. See [Editor themes](../editor-themes.md).
- Completion does not compile, run, re-lex, or access network/storage. Builtins come from the symbol
  catalog and signatures from `script_manager::signature`. Add new builtins to both tables.
  `array.*` and `map.*` names are offered straight from `array_builtins::KNOWN` / `map_builtins::NAMES`
  (and their signatures from `parameters()`), so a new one needs no editor edit.
  The suggestion list shows `COMPLETION_ROWS` (10) rows and scrolls to the highlight on ↑/↓: the key
  events are consumed by `completion_keys` before the list is drawn, so it sets
  `CompletionView::scroll_to_selected` instead of reading `key_pressed`. Accepting an `Item` with
  `callable` set (builtin functions, user `name(args) =>` definitions) writes `name()` with the caret
  between the parentheses, unless a `(` already follows.
- `script_manager::diff` is the sole saved-vs-source comparison. Cache `EditorState::changes()` for
  gutter, status, popover, and revert; never diff while painting. Hunks use zero-based half-open
  ranges and preserve terminators. Deleted hunks sit at `new_lines.start`, without fake rows;
  `is_dirty()` remains a string comparison.
- `revert_edit` must reject stale text. Apply through `apply_hunk_edit` so revert is one undo step.
  Only `mark_saved`/`replace_source(saved)` move the baseline. Use `theme::editor_diff_*` colors.
- `script_manager::symbols` resolves names for go-to-definition and rename. It reads lexer tokens, not
  the AST (which has no span for parameters, loop variables, tuple names or aliases and drops every
  statement it cannot parse), so it works on a script mid-edit. Scopes come from each statement's raw
  leading whitespace; a declaration is visible from the end of its statement and block; values and
  callables are separate namespaces; members, fields and `lib.fn` are not resolved. Cache
  `EditorState::symbols()` by text hash. A rename is never reasoned about: `SymbolTable::rename` applies
  it, analyses the result again and refuses unless every name still resolves to the same declaration.
  Test it with CRLF and astral characters; every range is a char index.
- Multiple carets: egui's text field keeps the *primary* caret (it draws and moves it);
  `editor/carets.rs` holds the others in `CaretState::extra`. While extras exist, `carets::apply` takes
  the editing and moving keys before egui and runs them through pure `script_manager::multi_cursor`,
  which reuses `edit_ops` per caret against one shared scan and lays the edits down together (an edit
  overlapping an earlier one drops its caret). Extras belong to the text they were made in: any other
  change (egui typing, undo, find/replace, rename, sync) fails the `hash` check and drops them, which is
  also how undo collapses them. Do not give extras their own undo history. Escape blurs an egui text
  field before any editor code runs, so `apply` reads it through `had_focus_last_frame` and re-takes the
  focus. Clicks are seen before egui moves the caret for them (`note_pointer`) and finished against the
  laid-out text afterwards (`resolve`); ⌘/Ctrl-click is a jump when the click is on a name with a
  definition and a new caret otherwise, ⌥-click always a new caret.
- The suggestion list and the signature panel are `egui::Area`s on their own layer, which the editor's clip
  rect does not reach, and they follow `caret_rect` as the text scrolls. `EditorState::caret_hidden` (the caret
  is outside the visible text area, set in `show`) hides both, and drops the list so Enter/Tab cannot accept an
  invisible row. Do not use `Area::constrain_to`: it pins the popup to the edge instead of hiding it.
- A jump's flash is `Flash`: the character it landed on, or (`show_line`: profile hot lines, debugger frames)
  the whole line as a band behind the text that holds, then fades over 700ms.
- The right-click menu, the ⌘-hover cue and the inline rename box are `editor/navigation.rs`. An egui
  `Area` takes no focus on its first frame (it only measures), so the rename field asks for it on the
  second.

## Drawer tabs

- The preview drawer's Report, Problems and Profile tabs share `script_manager::tab_chrome`: a 46px
  toolbar with a `state_field`, a hairline, then cards of rows. The Logs tab (`ui::pine_logs_view`,
  shared with the chart workspace) mirrors its constants; keep `TOOLBAR_HEIGHT`, `MARGIN` and
  `BUTTON` equal.
- Spend width before height: `chrome::wide` (520pt of room) puts parts in `chrome::columns`, and
  `pine_logs_view::intro` puts a picture beside its steps. Nothing outside a scroll area may be
  taller than the drawer (minimum 90pt): bodies use `chrome::scroll`, empty states use `centered`
  (which scrolls), and scroll areas set `min_scrolled_height(0.0)` (egui's default of 64 pushes a
  tab past a short drawer). `drawer_tabs_tests` asserts every tab uses no more than the room at five
  sizes; set `DEGENVIEW_GALLERY_DIR` to get pictures.
- A tab with nothing to show says so with `centered` + `pine_logs_view::intro` (picture, three steps,
  an example; see `drawer_intro`), never a bare text placeholder. That covers Inputs too
  (`drawer_intro::inputs`); `drawer_tabs_tests::inputs_fit_the_drawer` checks it at the five sizes.
- The Problems tab's copy button copies every problem (errors first); a right-click on a row copies that
  problem or only its message.
- `pine_report::show` / `report_content` (chart settings) keep the compact look; the drawer lays out
  the same pieces (`metric_grid`, `equity_curve`) itself.

## Manager lifecycle

- **Scripts edited outside the app** (a person's editor, a coding agent working in the Scripts folder)
  reach the open editor through `script_manager::external` (a pure state machine; `mod.rs` does the
  I/O, `disk_banner` draws the question). The sidebar already followed the folder (1.5 s rescan); the
  open script's *text* now does too: every ~400 ms only the file's metadata is read
  (`ScriptStore::file_stamp`), and a moved file, or one still settling, triggers a rescan and
  `reconcile_open`. A change is acted on only after the same text has been seen twice, 300 ms apart (a
  file another tool is still writing is never loaded half-written). A **clean editor** shows the file's
  text through `editor::apply_script` (one undo step) and `mark_saved`, with a "Reloaded" note in the
  status strip. An editor with **unsaved typing is never overwritten**: a banner asks *Reload from disk*
  (undoable) or *Keep my version* (`rebase_saved`: the typing stays and the file's text becomes the saved
  baseline, so a later Save overwrites on purpose), and Save is held while the question is open.
  **Saving is checked at the store**: `save_open` uses `ScriptStore::save_if_unchanged(id, source,
  saved_source)`, so a file rewritten after the last look is never overwritten silently (the next look
  then asks at once, `Watch::force`). Equality ignores a missing final newline (`external::same_text`).
  Not covered: chart instances keep the source they were added with (as after any Save), an outside edit
  makes no revision, and a draft of a script that is not open is not protected from an outside edit.
- `ScriptManager` owns its window. `workspace::PreviewState` drives one crypto/stock preview with
  debounced apply and stale output; it has no alert coordinator. `script_manager::inputs` renders a
  precompiled `InputSchema` and is shared with chart settings.
- A script can fail at run time after it compiles (the compiler judges some names only when they run).
  `script_manager::problems` is the one list of what is wrong: the compile diagnostics plus the chart's
  and the profile's failure, each only if its run was of the text now in the editor and never beside a
  compile error. The Problems tab and its badge, the editor's chip and the red underline all read it, so
  a failure must not be handled in one place only. `ExecutionController` remembers its last failure
  (`last_error`) and an update that changes nothing repeats it: it used to report a failure once and then
  read as a success.
- **A new builtin is registered in `pine::registry`.** The registry gathers the per-module name lists
  (`array_builtins::KNOWN`, `math::SUPPORTED`, `drawings::NAMESPACES`, `strategy_builtins::NAMES`, …) plus
  the few names the session evaluates inline (`TA_INLINE`, `STR_INLINE`, `OTHER_FUNCTIONS`), and the
  compiler and the editor's completion read it. The module tables stay what the interpreter dispatches
  from; add the name there **and**, if it is evaluated inline, to the registry. Two guards fail until you
  do: every registered function runs and is not `PINE4007` (registry ⊆ runtime), and every `"ns.member"`
  literal under `src/pine` in a closed namespace is registered or a known value (runtime ⊆ registry).
- In a **closed** namespace (`registry::CLOSED`) an unregistered call is `PINE3053`, an error, with a "Did
  you mean". In a **value-closed** one (`CLOSED_VALUES`) an unknown variable read is `PINE3050`. A
  namespace is added only when both guards pass and the Pine corpus is unchanged: `request` stays open
  (it has its own `PINE9003`); `syminfo`, `strategy` and `runtime` stay open for values because Pine adds
  variables there over time (`syminfo.cftc_code` failed the corpus). A false error on a script that runs is
  worse than a missed typo.
- **Builtin calls are routed, not searched for.** `Session::call` used to find a builtin by asking each
  family in turn on every execution (about 100 string tests before `ta.sma`, 180 before `plot`). When the
  session is built, `call_binding::bind` sets `Expression::Call::route` on every call that nothing in the
  program can capture (no function, method, type or import alias of that name or head), and `call_routed`
  (`call_routes.rs`) goes straight to the family. The chain stays for everything else (user functions and
  methods, variable receivers like `levels.push`, `request.*`, `log.*`, `ticker.*`, debugger watch
  expressions, which are parsed separately and never bound) and as the backstop that raises `PINE4007`.
  It is one implementation with two entrances: each family is a method both the chain and the route call.
  **A new builtin goes in a family; a new family needs a `Route`, an arm in `route_of` in the chain's
  order, and an arm in `call_routed`.** Fail-safe: a family that returns `None` hands the call back to the
  chain, so a wrong route costs time, not correctness. The tests that guard it: `call_binding_tests`
  (every registered function run both ways with four argument shapes, the workloads both ways, user
  methods/types/aliases that must stay on the chain) and `PINE_CORPUS_DUAL=1` on the corpus (every
  community script both ways). A change here is checked with `tools/pine-bench.sh`; the workloads that
  show it are `many_ta`, `many_math`, `many_str`, `plots_and_drawings`, `containers`, `map_counting`.
- A script that fails when it runs has no profile (`Profiler::finish`), only the failure, which the
  Problems list also shows.
- Never close a Script Manager tab or quit over unsaved editor text or retained drafts without the
  `unsaved` flow. Tab close starts `begin_exit(CloseTab)` and cancels close; app quit/update restart
  use `DegenViewApp::request_quit` plus the native termination intercept. A failed save cancels exit.
  Don't Save restores saved text and calls `ScriptStore::discard_draft`.
- `script_manager::demo` seeds examples once and on demand from an empty state. Deleted demos stay
  deleted. Names equal filenames without a “Demo” prefix. Examples must be original GPL-compatible
  work and compile with zero diagnostics, including warnings.

`src/pine/corpus_tests.rs` uses the optional cached community corpus in `tools/pine-corpus/`. Reduce
failures with original code; never copy corpus scripts into the repository.

## iCloud Sync

User scripts sync as files through `sync::scripts::ScriptSink` (`StoreScriptSink` over `ScriptStore`).
Drafts and revisions never sync, unmodified bundled examples are neither uploaded nor kept over an
incoming script of the same name, and text being typed is never overwritten (the incoming version
waits in the sync inbox). Two edits to one script keep both: the loser becomes a conflict-copy
script. Never write script files from sync code except through the sink. See
[iCloud Sync](../icloud-sync.md).
