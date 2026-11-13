# AGENTS.md — DegenView Rust

Guidance for AI coding agents working in this repository.

## Required reading

Always read [Core guidance](docs/agent-guides/core.md), then every guide whose scenario matches:

| Scenario | Read |
| --- | --- |
| Providers, symbols, timeframes, prediction markets, caches, icons, live data, or watchlists | [Market data](docs/agent-guides/market-data.md) |
| Live trading, brokers, broker credentials, or Settings ▸ Brokers | [Live trading](docs/broker-trading.md) |
| The symbol picker (⌘K, Add Chart and a watchlist's plus: markets, prediction markets, indices and portfolio cards), symbol search ranking, or how markets are named | [Symbol picker](docs/symbol-picker.md) |
| Any UI: a new or restyled window, sheet, panel, card, sidebar, icon, menu, button, link or dialog | [UI style](docs/agent-guides/ui-style.md) |
| Chart rendering, price labels, chart types, canvas, templates, drawings, portfolio cards, or snapshots | [Charts](docs/agent-guides/charts.md) |
| Markets compared on a chart (Compare, overlays), their data, scales, legend or the picker's compare mode | [Chart overlays](docs/chart-overlays.md) |
| The Portfolio tab: Overview, Holdings, Transactions, Statistics, their empty states, or portfolio revaluation | [Portfolio page](docs/portfolio-page.md) |
| Text, note, table, image or post drawings | [Annotation tools](docs/annotation-tools.md) |
| Chart patterns, Elliott waves or cycle drawings | [Chart patterns, Elliott waves and cycles](docs/pattern-tools.md) |
| Position, forecast, volume profile, VWAP or measurement drawings | [Forecasting, volume and measurement tools](docs/forecast-tools.md) |
| Fibonacci or Gann drawings, level lists, fans, circles, arcs, spirals or Gann angles | [Fibonacci and Gann tools](docs/fibonacci-gann-tools.md) |
| Session statistics: gap fills, prior-day breaks, weekday effects, `degenview_get_session_stats` | [Session statistics](docs/session-stats.md) |
| Pine language/runtime, Pine Studio, editor behavior, or example scripts | [Pine and scripts](docs/agent-guides/pine.md) |
| Adding an indicator to a chart: the toolbar's Indicators button, the indicator catalog, or the Pine indicators bundled with the app | [Indicator catalog](docs/indicator-picker.md) |
| User-defined functions or methods, overloads, call resolution, or the Pine type checker | [User-defined methods](docs/pine-user-methods.md) |
| Editor colours, syntax roles, editor themes or fonts | [Editor themes](docs/editor-themes.md) |
| Colour swatches in the Pine editor, the inline colour picker, or `color.*` rewriting | [Pine colour picker](docs/pine-color-picker.md) |
| `request.footprint()`, `footprint` or `volume_row` | [Pine volume footprints](docs/pine-footprint.md) |
| The Pine debugger: breakpoints, stepping, watches, the interpreter's debug hooks, `pine/debug/` | [Pine debugger](docs/pine-debugger.md) |
| The Pine profiler: the recorder, regions, profile sessions, the profile column and tab | [Pine profiler](docs/pine-profiler.md) |
| The top toolbar: its buttons, labels collapsing to icons, undo/redo, replay or chart-type controls | [Workspace toolbar](docs/workspace-toolbar.md) |
| The AI assistant, LLM providers, or the AI API keys | [AI assistant](docs/ai-assistant.md) |
| Price/Pine alerts or webhooks | [Alerts and webhooks](docs/agent-guides/alerts-webhooks.md) |
| Tab Linking: colour-tagged tabs that share symbol, interval, time or date range across windows | [Tab Linking](docs/tab-linking.md) |
| Tabs, windows, workspaces, saved layouts, session state, or app-wide state | [Tabs and state](docs/agent-guides/tabs-state.md) |
| MCP server, tools, permissions, or clients | [MCP](docs/agent-guides/mcp.md) |
| Releases, signing, packaging, self-update, or update UI | [Releases and updates](docs/agent-guides/releases-updates.md) |
| The Flatpak, Linux packaging, or the sandbox | [Flatpak](docs/flatpak.md) |
| Persisting user data: SQLite tables, `setting` keys, saved state, per-Mac IDs | [Storage and sync](docs/agent-guides/storage.md) |
| iCloud Sync, the sync outbox, or CloudKit | [iCloud Sync](docs/icloud-sync.md) |

For cross-cutting work, load all matching guides. They hold implementation constraints, not
background; never skip one because the code looks self-explanatory.

## Never blue

Links are always gray (`theme::text_secondary()`, underlined); the blue accent is not used for new
UI: no links, buttons or decoration. Details and existing exceptions:
[UI style](docs/agent-guides/ui-style.md).

## Flat style

Flat surfaces: faint fill and hairline; no gradients, lit edges or shadows (except floating menus and
dialogs). Icons bare, no tile, disc or circle. Colour only for state and data. Settings pages have no
bottom bar or Save button: inputs save as entered. Secondary windows keep the platform title bar and
window controls. Reference: the Settings window; rules and helpers in
[UI style](docs/agent-guides/ui-style.md).

## Menus fit their rows

A pull-down is exactly as wide as its longest row. Never guess a width (`set_min_width(180.0)`): use
`kit::fitted_menu` (or `kit::menu_fit`), and look at the opened menu in a picture before shipping.
Details: [UI style](docs/agent-guides/ui-style.md).

## Supply chain

Nothing reaches a release that was not reviewed, pinned and built from a signed tag on `main`.
When touching `.github/`, `tools/`, `packaging/`, `build.rs`, `Cargo.toml`/`Cargo.lock` or the
updater (`src/service/update/`): pin every `uses:` to a commit,
keep job permissions minimal, never run third-party code on a macOS release job after the signing
certificate is imported, use `--locked`, add no git or non-crates.io dependency, keep
`cargo deny check` clean. Never ask for, store or use a GitHub token with `admin:ssh_signing_key`,
`admin:*` or `delete_repo` scope; never push a tag. Details:
[Releases and updates](docs/agent-guides/releases-updates.md#build-hardening).

## Choosing the release type

When a task ends with "should we release this?", or the user asks what to release, check changes
since the last tag (`git tag --sort=-v:refname | head -3`, then `git log v<last>..HEAD`) and suggest
one row below with the resulting version. Never run `tools/release.sh` unless the user picked the
version. Procedure, channels and scheme:
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

Say which rows matched and why; when two match, prefer the safer. Tags are `v` plus the version;
prereleases are `-rc.N` (dotted, numbered from 1). A release candidate is never promoted: the final
version is a fresh tag.

## Running tests

The Pine corpus test (`src/pine/tests/corpus.rs`, ~295 community scripts) is most of the ~5 min
unit-test wall-clock, so it sits behind the `corpus` cargo feature, off by default:

- `cargo test` — fast suite, corpus excluded.
- `cargo test --features corpus` — full suite including the Pine corpus.

## Worktrees and build space

Each worktree builds into its own `target/` (5 GB fresh, growing every rebuild; cargo never deletes
old output). Finished worktrees are where the disk goes.

- One worktree per branch, beside the main checkout, building into `target/` or a `target-*`
  directory (both ignored). Never share one `CARGO_TARGET_DIR` between worktrees: concurrent builds
  block on its lock and overwrite each other's `target/debug/degenview`.
- Remove a worktree when its pull request merges. `tools/clean-worktrees.sh` lists every worktree and
  whether it can go; `--remove-merged` removes finished ones, never one with uncommitted changes,
  newer commits, no pull request, or a process running from it. `--clean-idle DAYS` deletes only the
  `target/debug` of worktrees not built for that long.
- `[profile.dev]` keeps line tables but drops variable-level debug info (about a third of a fresh
  `target/`; the incremental cache is the rest of the growth). To inspect your code's variables in
  lldb, build with `CARGO_PROFILE_DEV_DEBUG=true` into a separate `CARGO_TARGET_DIR`.
- A full disk shows as `rustc-LLVM ERROR: IO failure on output stream: No space left on device`.
  Check `df -h` and run the script first.

## Deeper references

- [Architecture](docs/architecture.md) — module map, data flow.
- [Chart types](docs/chart-types.md) — construction rules, Pine semantics.
- [Chart navigation](docs/chart-navigation.md) — viewport, pan/axis gestures, older history, unverified items.
- [Price scale modes](docs/price-scale.md) — Regular, Percent, Indexed to 100, Logarithmic; reference price, ticks, corner menu, persistence.
- [Linked charts](docs/chart-sync.md) — position/zoom sync between a workspace's charts.
- [Chart canvas](docs/chart-canvas.md) — settings, geometry, layering, templates.
- [Chart context menus](docs/chart-context-menu.md) — right-click menus, Table view, Object tree, what is verified.
- [Drawing tools](docs/drawing-tools.md) — tools, geometry, verified vs inferred behavior, limits.
- [Pine execution-model audit](docs/reviews/pine-execution-model-audit.md) — engine vs TradingView's execution model: findings, stacked fixes, what is deferred.
- [Pine Logs](docs/pine-logs.md) — `log.*()` messages: sink, limits, batches, TradingView verification.
- [Portfolio cards](docs/portfolio-cards.md) — card design, known gaps.
- [New-tab page](docs/new-tab-page.md) — the blank tab: search launcher, tabs, sparklines, animated backdrop.
- [Chart snapshots](docs/snapshots.md) — camera menu, shared renderer, hosted sharing via providers (ImgBB, your own host, adding one), privacy.
- [MCP implementation](docs/mcp-implementation.md) and [remaining work](docs/mcp-remaining-work.md).
- Also in Required reading, with details and what is verified versus decided:
  [Live trading](docs/broker-trading.md), [iCloud Sync](docs/icloud-sync.md),
  [Flatpak](docs/flatpak.md), [Chart overlays](docs/chart-overlays.md),
  [Tab Linking](docs/tab-linking.md), [Annotation tools](docs/annotation-tools.md),
  [Chart patterns, Elliott waves and cycles](docs/pattern-tools.md),
  [Forecasting, volume and measurement tools](docs/forecast-tools.md),
  [Session statistics](docs/session-stats.md), [Fibonacci and Gann tools](docs/fibonacci-gann-tools.md),
  [Symbol picker](docs/symbol-picker.md), [Workspace toolbar](docs/workspace-toolbar.md),
  [Indicator catalog](docs/indicator-picker.md), [Pine colour picker](docs/pine-color-picker.md),
  [Pine debugger](docs/pine-debugger.md), [Pine profiler](docs/pine-profiler.md),
  [Editor themes](docs/editor-themes.md), [User-defined methods](docs/pine-user-methods.md),
  [Pine volume footprints](docs/pine-footprint.md), [AI assistant](docs/ai-assistant.md),
  [Portfolio page](docs/portfolio-page.md).
