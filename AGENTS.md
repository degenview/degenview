# AGENTS.md — DegenView Rust

Guidance for AI coding agents working in this repository.

## Required reading

Read [Core guidance](docs/agent-guides/core.md) for every task. Then read every guide whose
scenario matches the work:

| Scenario | Read |
| --- | --- |
| Providers, symbols, timeframes, prediction markets, caches, icons, live data, or watchlists | [Market data](docs/agent-guides/market-data.md) |
| Chart rendering, price labels, chart types, canvas, templates, drawings, portfolio cards, or snapshots | [Charts](docs/agent-guides/charts.md) |
| Pine language/runtime, Script Manager, editor behavior, or example scripts | [Pine and scripts](docs/agent-guides/pine.md) |
| Price/Pine alerts or webhooks | [Alerts and webhooks](docs/agent-guides/alerts-webhooks.md) |
| Tabs, windows, workspaces, saved layouts, session state, or app-wide state | [Tabs and state](docs/agent-guides/tabs-state.md) |
| MCP server, tools, permissions, or clients | [MCP](docs/agent-guides/mcp.md) |
| Releases, signing, packaging, self-update, or update UI | [Releases and updates](docs/agent-guides/releases-updates.md) |
| iCloud Sync, the sync outbox, CloudKit, or anything that persists user data | [iCloud Sync](docs/icloud-sync.md) |

For cross-cutting work, load all matching guides. These files contain implementation constraints,
not general background; do not skip a guide because the code looks self-explanatory.

## Running tests

The Pine corpus test (`src/pine/corpus_tests.rs`) runs ~295 community scripts and accounts for most
of the ~5 min unit-test wall-clock. It is gated behind the `corpus` cargo feature, off by default:

- `cargo test` — fast suite, corpus excluded.
- `cargo test --features corpus` — full suite including the Pine corpus.

## Deeper references

- [Architecture](docs/architecture.md) — module map and data flow.
- [iCloud Sync](docs/icloud-sync.md) — what syncs, merge rules, account safety, Apple setup, what is unverified.
- [Chart types](docs/chart-types.md) — construction rules and Pine semantics.
- [Chart navigation](docs/chart-navigation.md) — viewport, pan/axis gestures, older history, what is unverified.
- [Linked charts](docs/chart-sync.md) — position/zoom sync between a workspace's charts.
- [Chart canvas](docs/chart-canvas.md) — settings, geometry, layering, and templates.
- [Chart context menus](docs/chart-context-menu.md) — right-click menus, Table view, Object tree, what is verified.
- [Drawing tools](docs/drawing-tools.md) — tools, geometry, verified versus inferred behavior, limits.
- [Portfolio cards](docs/portfolio-cards.md) — card design and known gaps.
- [Chart snapshots](docs/snapshots.md) — the camera menu, one shared renderer, hosted sharing protocol, privacy.
- [MCP implementation](docs/mcp-implementation.md) and [remaining work](docs/mcp-remaining-work.md).
