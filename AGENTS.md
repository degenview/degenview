# AGENTS.md — DegenView Rust

Guidance for AI coding agents working in this repository.

## Required reading

Read [Core guidance](docs/agent-guides/core.md) for every task. Then read every guide whose
scenario matches the work:

| Scenario | Read |
| --- | --- |
| Providers, symbols, timeframes, prediction markets, caches, icons, live data, or watchlists | [Market data](docs/agent-guides/market-data.md) |
| Chart rendering, price labels, chart types, canvas, templates, drawings, portfolio cards, or snapshots | [Charts](docs/agent-guides/charts.md) |
| Text, note, table, image or post drawings | [Annotation tools](docs/annotation-tools.md) |
| Chart patterns, Elliott waves or cycle drawings | [Chart patterns, Elliott waves and cycles](docs/pattern-tools.md) |
| Position, forecast, volume profile, VWAP or measurement drawings | [Forecasting, volume and measurement tools](docs/forecast-tools.md) |
| Fibonacci or Gann drawings, level lists, fans, circles, arcs, spirals or Gann angles | [Fibonacci and Gann tools](docs/fibonacci-gann-tools.md) |
| Pine language/runtime, Script Manager, editor behavior, or example scripts | [Pine and scripts](docs/agent-guides/pine.md) |
| `request.footprint()`, `footprint` or `volume_row` | [Pine volume footprints](docs/pine-footprint.md) |
| The Pine debugger: breakpoints, stepping, watches, the interpreter's debug hooks, `pine/debug/` | [Pine debugger](docs/pine-debugger.md) |
| The Pine profiler: the recorder, regions, profile sessions, the profile column and tab | [Pine profiler](docs/pine-profiler.md) |
| Price/Pine alerts or webhooks | [Alerts and webhooks](docs/agent-guides/alerts-webhooks.md) |
| Tabs, windows, workspaces, saved layouts, session state, or app-wide state | [Tabs and state](docs/agent-guides/tabs-state.md) |
| MCP server, tools, permissions, or clients | [MCP](docs/agent-guides/mcp.md) |
| Releases, signing, packaging, self-update, or update UI | [Releases and updates](docs/agent-guides/releases-updates.md) |
| iCloud Sync, the sync outbox, CloudKit, or anything that persists user data | [iCloud Sync](docs/icloud-sync.md) |

For cross-cutting work, load all matching guides. These files contain implementation constraints,
not general background; do not skip a guide because the code looks self-explanatory.

## Choosing the release type

When a task ends with "should we release this?", or the user asks what to release, look at what
changed since the last tag (`git tag --sort=-v:refname | head -3`, then `git log v<last>..HEAD`) and
suggest one row below, with the resulting version. Never run `tools/release.sh` unless the user has
picked the version. Procedure, channels and scheme:
[Releases and updates](docs/agent-guides/releases-updates.md).

| Changes since the last release | Suggest |
| --- | --- |
| Only bug fixes, copy or UI polish, docs, or performance with no behaviour change | PATCH, stable: `0.5.1` → `0.5.2` |
| A new feature or visible behaviour change, low risk | MINOR, stable: `0.6.0` |
| A new feature that touches the updater, signing or entitlements, iCloud/CloudKit records, the store schema, Pine runtime semantics, or anything that can lose user data | MINOR as a release candidate first: `0.6.0-rc.1`, then `0.6.0` after a clean RC |
| A breaking change to saved data or to the MCP or Pine public surface | MINOR (MAJOR from 1.0), release candidate first |
| A fix to a release candidate | The next candidate: `0.6.0-rc.2` |
| A fix needed on stable while a candidate is open | PATCH off stable (`0.5.2`), but `tools/release.sh` only cuts from `main`, which already holds the candidate's changes: tell the user, who needs a hotfix branch |
| Only docs, tests or tooling that no user build contains | No release |

Say which rows matched and why, and prefer the safer row when two match. Tags are `v` plus the
version, prereleases are `-rc.N` (dotted, numbered from 1), and a release candidate is never
promoted: the final version is a fresh tag.

## Running tests

The Pine corpus test (`src/pine/corpus_tests.rs`) runs ~295 community scripts and accounts for most
of the ~5 min unit-test wall-clock. It is gated behind the `corpus` cargo feature, off by default:

- `cargo test` — fast suite, corpus excluded.
- `cargo test --features corpus` — full suite including the Pine corpus.

## Worktrees and build space

Every worktree builds into its own `target/`, which is 5 GB fresh and grows with each rebuild
because cargo never deletes old output. Finished worktrees are where the disk goes.

- One worktree per branch, beside the main checkout. Keep its build output in `target/` or a
  `target-*` directory (both are ignored). Do not share one `CARGO_TARGET_DIR` between worktrees:
  concurrent builds would block on its lock and overwrite each other's `target/debug/degenview`.
- When a pull request merges, remove its worktree. `tools/clean-worktrees.sh` lists every worktree and
  whether it can go; `--remove-merged` removes the finished ones, and never one with uncommitted
  changes, newer commits, no pull request, or a process running from it. `--clean-idle DAYS` deletes
  only the `target/debug` of worktrees not built for that long.
- `[profile.dev]` keeps line tables but drops variable-level debug info, about a third of a fresh
  `target/` (the incremental cache is the rest of what grows). To inspect your own code's variables in lldb, build with `CARGO_PROFILE_DEV_DEBUG=true`
  into a separate `CARGO_TARGET_DIR`.
- A full disk shows as `rustc-LLVM ERROR: IO failure on output stream: No space left on device`.
  Check `df -h` and run the script before anything else.

## Deeper references

- [Architecture](docs/architecture.md) — module map and data flow.
- [iCloud Sync](docs/icloud-sync.md) — what syncs, merge rules, account safety, Apple setup, what is unverified.
- [Chart types](docs/chart-types.md) — construction rules and Pine semantics.
- [Chart navigation](docs/chart-navigation.md) — viewport, pan/axis gestures, older history, what is unverified.
- [Price scale modes](docs/price-scale.md) — Regular, Percent, Indexed to 100, Logarithmic; reference price, ticks, corner menu, persistence.
- [Linked charts](docs/chart-sync.md) — position/zoom sync between a workspace's charts.
- [Chart canvas](docs/chart-canvas.md) — settings, geometry, layering, and templates.
- [Chart context menus](docs/chart-context-menu.md) — right-click menus, Table view, Object tree, what is verified.
- [Drawing tools](docs/drawing-tools.md) — tools, geometry, verified versus inferred behavior, limits.
- [Annotation tools](docs/annotation-tools.md) — text, notes, tables, images and posts; assets and previews.
- [Chart patterns, Elliott waves and cycles](docs/pattern-tools.md) — the Patterns tile: ratios, labels and degrees, cycles counted in bars, provenance.
- [Forecasting, volume and measurement tools](docs/forecast-tools.md) — the Forecast tile: hypothetical positions, ghost feed, VWAP and volume profiles from the chart's own bars, measurers, provenance.
- [Fibonacci and Gann tools](docs/fibonacci-gann-tools.md) — the Fibonacci and Gann tile: editable level lists, bar-counted time lines, curves sampled where they show, Gann angles as ratios of price to time, provenance.
- [Pine debugger](docs/pine-debugger.md) — the opt-in interpreter hooks, sessions on their own thread, source map and stepping, breakpoints, the read-only expression service, what is measured.
- [Pine profiler](docs/pine-profiler.md) — the timing model (regions, inclusive and exclusive time, what is not attributed), the opt-in recorder, profile sessions and staleness, the editor column and drawer tab, what is measured.
- [Pine volume footprints](docs/pine-footprint.md) — `request.footprint()`: built from intrabar candles, stateless handles, what is inferred and what is not verified.
- [Pine Logs](docs/pine-logs.md) — `log.*()` messages: the sink, limits, batches, what is verified against TradingView.
- [Portfolio cards](docs/portfolio-cards.md) — card design and known gaps.
- [Chart snapshots](docs/snapshots.md) — the camera menu, one shared renderer, hosted sharing protocol, privacy.
- [MCP implementation](docs/mcp-implementation.md) and [remaining work](docs/mcp-remaining-work.md).
