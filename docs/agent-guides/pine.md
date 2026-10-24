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
- Breakpoints and watches live in the local `scriptDebug.state` setting (never synced; listed in
  `sync::policy`'s local keys).
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
- `script_manager::diff` is the sole saved-vs-source comparison. Cache `EditorState::changes()` for
  gutter, status, popover, and revert; never diff while painting. Hunks use zero-based half-open
  ranges and preserve terminators. Deleted hunks sit at `new_lines.start`, without fake rows;
  `is_dirty()` remains a string comparison.
- `revert_edit` must reject stale text. Apply through `apply_hunk_edit` so revert is one undo step.
  Only `mark_saved`/`replace_source(saved)` move the baseline. Use `theme::editor_diff_*` colors.

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
