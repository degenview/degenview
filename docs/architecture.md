# DegenView Rust architecture

DegenView is a Rust/egui market dashboard with a small native-window boundary. This page is the
architecture index; use the topic documents for detail.

## Layers

| Layer | Responsibility |
| --- | --- |
| `app.rs`, `workspace.rs`, `host/` | Stateful coordination and feature ownership |
| `model/` | Pure domain values and engines |
| `service/` | Providers, network, caches, Keychain, and OS services |
| `store/` | SQLite persistence and Swift import |
| `ui/` | Immediate-mode egui rendering |
| `pine/` | Pure Pine compiler, interpreter, broker, and outputs |
| `script_manager/` | Script library, editor, preview, and drafts |
| `mcp/`, `host/mcp_host.rs` | Local MCP transport and app-thread bridge |

## Topic map

| Work | Read |
| --- | --- |
| Module ownership or placement | [Modules](architecture/modules.md) |
| Startup, windows, concurrency, persistence, portfolios, or paper trading | [Runtime and data flow](architecture/runtime.md) |
| Chart canvas, indicators, drawings, undo, or replay | [Charts and replay](architecture/charts.md) |
| Pine compilation/execution or the Script Manager | [Pine](architecture/pine.md) |
| MCP transport, permissions, tools, or source editing | [MCP server](architecture/mcp.md) |
| Price/Pine alerts and webhook delivery | [Alerts and webhooks](architecture/alerts-webhooks.md) |
| Memory/disk caches and cache clearing | [Cache](architecture/cache.md) |

For feature-specific behavior, also see [chart types](chart-types.md),
[chart canvas](chart-canvas.md), and [portfolio cards](portfolio-cards.md).
