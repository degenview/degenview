# Architecture refactor (November 2026)

A review of `src/` (896 files, about 470k lines, one crate) against the rules in
[Core guidance](../agent-guides/core.md) and [Modules](../architecture/modules.md), and the plan
that follows from it. Each item is checked off, with its pull request, as it lands. This is a
dated work record, not guidance: once every box is ticked the rules live in the guides it changed.

## Findings

What holds up:

- Layers are documented and directory-per-domain naming is enforced by `src/layout_tests.rs`.
- `model/` and `pine/` draw nothing; `pine/` needs only `model::Kline` and a few chart types.
- `sync/engine/core.rs` is a generic `SyncCore<T: SyncTransport>` that emits effects.
- Production code has few `unwrap`s, `unsafe` only at FFI boundaries with SAFETY comments, a shared
  worker pool (`service::work_queue`), and an MCP permission check that fails closed.

What does not:

1. **Two god structs.** `ChartWorkspace` has 87 fields, its `impl` spread over 28 files that each
   see every field; `DegenViewApp` has 55, 19 of them `Arc<Mutex<_>>`. Splitting an `impl` over
   files is not decoupling: nothing has an interface and nothing can be tested alone.
2. **Views in hosts, an engine in a view.** About 1,700 lines of egui live in `host/portfolio`
   (`PortfolioTab`) and `host/paper` (`PaperPanel`). In the other direction a whole Pine execution
   worker lives in the chart card (`ui/chart/card`), and its `request.security` provider fetches
   candles over the network.
3. **UI doing I/O.** Nine `ui/` files spawn their own threads (symbol and asset search skip the
   provider rate-limit lanes of `work_queue::submit_for`), Keychain reads run on the UI thread, and
   the Settings window queries SQLite and parses JSON every frame.
4. **Errors as strings.** No error derive crate, about 290 `Result<_, String>`; `fetch_klines`
   turns `http::Error::Status(429)` into text, so the chart card matches `contains("429")`. The sync
   engine uses `rusqlite::Error` as its own error type.
5. **Lint blankets.** `#![allow(dead_code)]` on 61 files and module-wide on `pine/`,
   `pine_studio/`, `sync/` and `store/sync/`, with reasons ("lands in stages") that no longer hold.
   CI runs neither Clippy nor rustfmt.
6. **MCP plumbing.** A new tool touches about eight places; the app's `run_mcp_command` is one
   540-line `match`; `mcp/tools.rs` holds about 30 handlers inline and seven copies of id parsing.
7. **Dependencies pointing the wrong way.** `model → service`, `model → pine_studio`,
   `model → sync`, `mcp → ui` (MCP drives the alert editor's UI state), `sync → host`, and a
   `pine ↔ model` cycle that blocks splitting the engine into its own crate.
8. **Big mixed files.** 19 files over 2,000 lines; the worst mix concerns (`ui/chart/card`,
   `pine/runtime/session.rs`, `pine_studio/editor`, `store/sync`, `ui/kit`, `ui/watchlist/sidebar.rs`,
   `ui/settings/mcp.rs`). About 130 files keep inline `mod tests` instead of `tests.rs`.
9. **Copies.** The asset logo and monogram five times, the start-and-poll background job five times
   (beside `trading::task::Task`), a hairline divider six times, `with_alpha` three times,
   `empty_state` seven times.

## Plan

Stacked pull requests, logic changes first and pure moves last.

### Wave 1: guardrails

- [x] **PR 1: lint hygiene.** [#511](https://github.com/degenview/degenview/pull/511)
  - [x] Remove the module-wide `allow(dead_code)`/`allow(unused_imports)` in `pine/`, `pine_studio/`,
        `sync/`, `store/sync/`.
  - [x] Remove the 61 file-level `#![allow(dead_code)]`; delete what is dead, keep a narrow, reasoned
        `#[allow]` only for code that is used on another platform or feature.
  - [x] `parse_id` in `mcp/tools.rs` names the tool that returns the id it parses.
- [x] **PR 2: layering guard and CI lints.** [#512](https://github.com/degenview/degenview/pull/512)
  - [x] `layout_tests.rs` checks import direction (`model`/`pine` import no coordinator, UI or I/O;
        `service`/`store`/`sync` import no `ui`/`host`/`app`/`workspace`; `ui` imports no `app`), with
        today's exceptions in an allowlist that later PRs empty. A second test bans drawing types
        (`egui::Ui`, windows, panels) outside the view layers, with the four host files in its debt
        list.
  - [x] CI runs `cargo fmt --check` and `cargo clippy --all-targets --all-features -D warnings`.

### Wave 2: layering

- [x] **PR 3: views out of hosts.** [#514](https://github.com/degenview/degenview/pull/514)
  - [x] `PortfolioTab` and the portfolio bar and import sheets move to `ui/portfolio/`.
  - [x] `PaperPanel` and its banners move to `ui/paper/`; the export runs on the work queue.
  - [x] `host/webhook` becomes `host/settings`, the Settings-window controller it is.
- [x] **PR 4: the Pine worker leaves the chart card.** [#515](https://github.com/degenview/degenview/pull/515)
  - [x] The chart card's worker and run bookkeeping move to `host::pine`; the card asks for a run and
        keeps the latest output.
  - [x] Indicator-picker previews run through the same host.
- [x] **PR 5: one background-job handle.** [#516](https://github.com/degenview/degenview/pull/516)
  - [x] `service::work_queue::Pending<T>` (and `PendingAll<T>`), which repaints on completion.
        `trading::task::Task` stays: venues answer through polling closures (paper at once, brokers
        through the broker crate's own handle), not through one job.
  - [x] UI thread spawns (searches, test connections, uploads, file dialogs) go through it; searches use
        the per-provider lanes; Keychain reads move onto the job.
  - [x] Pine Studio's preview fetch, search and export use it too.
- [x] **PR 6: types where they belong.** [#517](https://github.com/degenview/degenview/pull/517)
  - [x] CoinMarketCap chart-type titles move to `model::market`.
  - [x] `ReplayCommand`/`ReplayStart` move to `model`.
  - [x] A pure `model::alert::draft` serves both the alert editor and MCP.
  - [x] `LocalScript`, `ScriptType` and the bundled-indicator type move to `model::pine`.
  - [x] Sync id-space constants and the custom-title limit move to `model`.
  - [x] `EventSink`/`TransportFactory` move to `sync::transport`; `StoreScriptSink` to `host::sync`.
  - [x] The script library's file store moves out of `pine_studio/`.
- [x] **PR 7: nothing slow in a frame.** [#518](https://github.com/degenview/degenview/pull/518)
  - [x] Pine subscriptions are cached by change signal instead of queried per frame.
  - [x] The MCP connection JSON is made only when copied; the local UTC offset is cached.

### Wave 3: structure

- [x] **PR 8: typed errors.** [#519](https://github.com/degenview/degenview/pull/519)
  - [x] Add `thiserror`.
  - [x] Providers return a `ProviderError`; no error is matched by its text.
  - [x] The sync engine has its own `SyncError`.
  - [x] One shared poison-tolerant `lock` helper.
- [x] **PR 9: `ChartWorkspace` in owned parts.** Drawing tools, layout session, watchlist pane, trade
      side, replay, undo history and alert UI become sub-structs with their own methods.
- [ ] **PR 10: `DegenViewApp` in owned parts.** An alert coordinator, the tab state and the MCP state.
- [ ] **PR 11: MCP routing.**
  - [ ] `McpCommand` splits into one sub-enum per domain; each domain's arms run in
        `host/mcp/<domain>.rs` and the app only routes.
  - [ ] `mcp/tools.rs` splits by domain; one client extractor, one `parse_id`, one `parse_color`.

### Wave 4: splits and copies

- [ ] **PR 12: one UI kit.**
  - [ ] Asset logo, divider, settings fields and buttons, card frames into `ui/kit`; local copies of
        `with_alpha`, `empty_state`, `time_text` and `galley` deleted.
  - [ ] `ui/kit/mod.rs` split by widget family.
- [ ] **PR 13: big files split** (move only), inline tests into `tests.rs`.
  - [ ] UI: chart card, watchlist sidebar, Settings ▸ MCP, portfolio overview/statistics, chart
        settings, tab bar, drawing properties.
  - [ ] Pine: runtime session, outputs/drawings; Pine Studio: editor, studio commands, preview.
  - [ ] Store and sync: `store/sync`, `sync/engine/apply`; `mcp/pine`.

### Later (not in this series)

- A workspace split: a small core crate, then the Pine engine on its own (needs PR 6). Before it,
  `store::scripts` must stop importing `pine_studio::library::{name, registry}`.
- `Store` as per-domain repositories with a separate read connection.
- One Pine builtin signature table for the registry, the type checker and the studio.
- Pine runtime allocations: overload lookups and `ta` windows that allocate on every bar.
