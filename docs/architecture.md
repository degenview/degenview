# DegenView Rust architecture

DegenView is a Rust/egui market dashboard with a small native-window boundary. This page is the
architecture index; use the topic documents for detail.

## Layers

| Layer | Responsibility |
| --- | --- |
| `app.rs`, `workspace/mod.rs`, `workspace/`, `host/` | Stateful coordination: the app, one chart tab (`workspace/` holds its concerns), and one host per feature |
| `model/` | Pure domain values and engines, one directory per domain |
| `service/` | Providers, network, caches, Keychain, and OS services |
| `store/` | SQLite persistence and Swift import |
| `sync/`, `native/swift/` | iCloud Sync: pure policy, merge and outbox logic, and the CloudKit bridge |
| `trading/` | Order entry for every venue (paper and brokers) behind one trait; brokers come from the `degenview-brokers` crate |
| `ui/` | Immediate-mode egui rendering |
| `pine/` | Pure Pine compiler, interpreter, strategy broker, debugger, profiler, and outputs |
| `pine_studio/` | Script library, editor, preview, assistant, debugger and profiler panels |
| `mcp/`, `host/mcp_host.rs` | Local MCP transport and app-thread bridge |
| `bin/` | The login-item alert agent (`degenview-alert-agent`) |

## Topic map

| Work | Read |
| --- | --- |
| Module ownership, placement or directory layout | [Modules](architecture/modules.md) |
| Startup, windows, concurrency, persistence, portfolios, or paper trading | [Runtime and data flow](architecture/runtime.md) |
| Chart canvas, indicators, drawings, undo, or replay | [Charts and replay](architecture/charts.md) |
| Pine compilation/execution or the Pine Studio | [Pine](architecture/pine.md) |
| MCP transport, permissions, tools, or source editing | [MCP server](architecture/mcp.md) |
| Price/Pine alerts and webhook delivery | [Alerts and webhooks](architecture/alerts-webhooks.md) |
| Memory/disk caches and cache clearing | [Cache](architecture/cache.md) |
| Live trading and brokers | [Live trading](broker-trading.md) |
| iCloud Sync | [iCloud Sync](icloud-sync.md), [release checklist](icloud-sync-release-checklist.md) |
| Flatpak and the Linux sandbox | [Flatpak](flatpak.md) |
| Releases, signing, self-update | [Releases and updates](agent-guides/releases-updates.md) |

## Feature references

Behavior, what is verified against TradingView and what is decided here, per feature. The root
`AGENTS.md` says which to read for a task.

- **Charts:** [chart types](chart-types.md), [canvas](chart-canvas.md), [navigation](chart-navigation.md),
  [price scale modes](price-scale.md), [linked charts](chart-sync.md), [overlays](chart-overlays.md),
  [context menus](chart-context-menu.md), [snapshots](snapshots.md), [symbol picker](symbol-picker.md),
  [toolbar](workspace-toolbar.md), [Tab Linking](tab-linking.md).
- **Drawings and analysis:** [drawing tools](drawing-tools.md), [annotations](annotation-tools.md),
  [patterns, Elliott waves and cycles](pattern-tools.md), [forecast, volume and measurement](forecast-tools.md),
  [Fibonacci and Gann](fibonacci-gann-tools.md), [session statistics](session-stats.md).
- **Portfolio:** [portfolio cards](portfolio-cards.md), [Portfolio page](portfolio-page.md).
- **Pine:** [indicator catalog](indicator-picker.md), [user-defined methods](pine-user-methods.md),
  [volume footprints](pine-footprint.md), [Pine Logs](pine-logs.md), [debugger](pine-debugger.md),
  [profiler](pine-profiler.md), [colour picker](pine-color-picker.md), [editor themes](editor-themes.md),
  [AI assistant](ai-assistant.md).
- **MCP:** [implementation](mcp-implementation.md), [remaining work](mcp-remaining-work.md).
- **Reviews and audits:** dated records in [`reviews/`](reviews/), not guidance.

[`context-menu.md`](context-menu.md) and [`profiler.md`](profiler.md) are task briefs written
as "Implement ..." prompts, not documentation of current behavior; use the feature docs above.
