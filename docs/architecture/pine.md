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

`pine_host::run_script` runs applied chart instances and returns renderer-neutral output.
`pine_subscription_host` runs active subscriptions and returns fired `alert()` events.
`request.*` uses `KlineSecurityDataProvider` with a short REST cache; imports use the Script
Manager's `LibraryRegistry`. `request.footprint()` builds a bar's volume footprint from the same
provider's finer candles ([Pine volume footprints](../pine-footprint.md)). `run_script_alerts`
replays the last bar as a realtime close.

`pine/debug/` adds an opt-in debugger without a second interpreter: a `Session` may carry a
`DebugRuntime` that its statement loop, user-function calls, bar start and `request.security`
evaluation consult. A `DebugSession` runs a private `ExecutionController` on its own thread over a
frozen copy of the preview's bars and talks to the UI through bounded channels. See
[Pine debugger](../pine-debugger.md).

## Script Manager

Scripts are file-backed (`Scripts/*.pine` plus `ScriptMetadata/`) with stable ids in extended
attributes. The manager owns the sidebar, editor, preview split, and Inputs/Report/Problems drawer.
Edits recompile for diagnostics. Drafts remain separate from saved source so invalid work cannot
replace the active saved result.
