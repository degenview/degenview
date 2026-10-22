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
