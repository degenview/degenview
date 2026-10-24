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
Manager's `LibraryRegistry`. `run_script_alerts` replays the last bar as a realtime close.

## Script Manager

Scripts are file-backed (`Scripts/*.pine` plus `ScriptMetadata/`) with stable ids in extended
attributes. The manager owns the sidebar, editor, preview split, and Inputs/Report/Problems drawer.
Edits recompile for diagnostics. Drafts remain separate from saved source so invalid work cannot
replace the active saved result.
