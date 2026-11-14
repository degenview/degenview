# Pine architecture

```text
source → Lexer → Parser → Compiler → CompiledProgram → Session
                           ▲                         ├→ VisualOutput
                    LibraryRegistry                ├→ strategy Broker
                                                   ├→ alert events
                                                   └→ log messages (LogBatch)
```

The compiler folds constants, collects inputs, validates structure/types, and links libraries.
`Session` interprets bars; builtin families route to focused pure modules. The strategy broker lives
inside per-bar state, so realtime rollback restores orders, positions, and trades too.
`pine/runtime/session/` holds the interpreter one concern per file: building a session (`build`),
running bars (`running`), `expressions`, `statements`, `calls` with the plain-value builtins
(`scalar`), the debugger's and profiler's `hooks`, and the `bookkeeping` between ticks and bars.

`host::pine::chart::ChartRuns` runs the scripts applied to a chart: the card hands it, each frame,
the bars it draws, the `syminfo`, theme and visible range, and its instances; every visible instance
gets a persistent `ExecutionController` on the work queue (`host/pine/chart/worker.rs`, latest
request wins), run again only when its source, inputs, market, bar length, theme or bars changed
(the visible range too, for a script that reads it). The card keeps only the outputs, failures,
logs and busy state it is handed back. The bars are hashed only when they differ from the copy the
runs were last handed (a bitwise comparison, cheaper than the hash). `host::pine::batch` runs a list
of scripts once each over the same bars (the indicator picker's previews), and
`host::pine::run_script_checked_in_range` runs one script once and returns renderer-neutral output.
`host::pine::subscription` runs active subscriptions and returns fired `alert()` events.
`request.*` uses `KlineSecurityDataProvider` with a short REST cache; imports use the Pine
Studio's `LibraryRegistry`. `request.footprint()` builds a bar's volume footprint from the same
provider's finer candles ([Pine volume footprints](../pine-footprint.md)). `run_script_alerts`
replays the last bar as a realtime close.

`pine/debug/` adds an opt-in debugger without a second interpreter: a `Session` may carry a
`DebugRuntime` that its statement loop, user-function calls, bar start and `request.security`
evaluation consult. A `DebugSession` runs a private `ExecutionController` on its own thread over a
frozen copy of the preview's bars and talks to the UI through bounded channels. See
[Pine debugger](../pine-debugger.md).

## Pine Studio

Scripts are file-backed (`Scripts/*.pine` plus `ScriptMetadata/`) with stable ids in extended
attributes. The manager owns the sidebar, editor, preview split, and Inputs/Report/Problems drawer;
the views queue `Cmd`s that it runs after the frame (`pine_studio/commands.rs`), the open script
and edits made outside the app are `open_script.rs`, and the Swift app's scripts and preview inputs
come across through `migrate_swift.rs`.
Edits recompile for diagnostics. Drafts remain separate from saved source so invalid work cannot
replace the active saved result. The editor resolves names itself for go to definition and
rename (`pine_studio/editor/symbols.rs`, token-based so it survives half-typed code) and supports
Sublime-style multiple carets (`pine_studio/editor/multi_cursor.rs`, `editor/carets.rs`).
