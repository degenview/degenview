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
  decide editor indentation because incomplete parentheses erase them.
- Completion does not compile, run, re-lex, or access network/storage. Builtins come from the symbol
  catalog and signatures from `script_manager::signature`. Add new builtins to both tables.
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
  an example; see `drawer_intro`), never a bare text placeholder.
- `pine_report::show` / `report_content` (chart settings) keep the compact look; the drawer lays out
  the same pieces (`metric_grid`, `equity_curve`) itself.

## Manager lifecycle

- `ScriptManager` owns its window. `workspace::PreviewState` drives one crypto/stock preview with
  debounced apply and stale output; it has no alert coordinator. `script_manager::inputs` renders a
  precompiled `InputSchema` and is shared with chart settings.
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
