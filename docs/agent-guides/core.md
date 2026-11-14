# Core Agent Guidance

Read for every task, plus the matching topic guides from the root `AGENTS.md` (UI work: [UI style](ui-style.md); persisted data: [Storage and sync](storage.md)).

## Project

DegenView is a GPL-3.0-only Rust market dashboard on `eframe`/`egui`. A small `objc2` AppKit bridge handles native macOS windows, menus, Keychain and accessibility. Keep the coordinator/workspace architecture on other platforms; gate AppKit code with `#[cfg(target_os = "macos")]`.

Store secrets only through `service::keychain::webhook_secrets::platform` (macOS Keychain, Linux Secret Service, Windows Credential Manager). Never in SQLite, files or logs.

Read [Architecture](../architecture.md) before changing module boundaries or data flow.

## Quality bar

- **Ship finished work.** Code and design are polished and production-ready when you hand them over: no stubs, TODOs, debug output, dead code or placeholder UI, and every state of a screen (empty, loading, error, long text) looks intended. "Works for now" is not done.
- **Decouple, even when it costs a bigger change.** Keep modules independent behind narrow interfaces: pure logic in `model/`, side effects behind hosts and services, no reaching across layers. If the clean design needs a larger change or a refactor of existing code, make it instead of a shortcut that couples more; when it is large, say so first.
- **Clean code is a priority.** Clear names, one concern per file and function, no copy-pasted logic (extract and reuse), comments only for the why, unused code deleted. Match the surrounding style.

## Architecture

- **Modules, not MVVM.** `model/` pure domain logic; `service/` providers/networking; `ui/` egui rendering; `store/` SQLite; `host/` (one directory per domain) bridges stateful components; `workspace/mod.rs` chart workspaces; `app/` coordinates.
- **Immediate-mode UI.** Pass state into views; never add observed view models. Render charts with `egui::Painter`, not a chart library.
- **Background work.** Outside `src/mcp/`, threads and `std::sync::mpsc`, no async runtimes. Shared state `Arc<Mutex<...>>`; keep locks short.
  Work a frame starts and waits on for one answer (a search, a test connection, a file dialog, an
  upload) runs through `service::work_queue::Pending`: `start`, `start_for(source, …)` (a provider's
  rate-limit lane), `start_search(source, …)` (a picker's search, off the candle workers) or `SNAPSHOTS.start` (anything that waits on the user: panels, Keychain prompts),
  polled each frame; the job repaints when it ends, and a panicked job ends without an answer
  (`poll_or`). Views hold the `Pending` a host or service returned; they never spawn threads or build
  channels.
- **Layering is tested.** `layout_tests.rs` fails when a module imports a layer above it (`model/` and
  `pine/` import no I/O, coordinator or view; `service/`, `store/`, `sync/` no `host/`, `ui/`, `app/` or
  `workspace/`) or when anything outside the view layers draws (`egui::Ui`, a window or a panel). Its
  debt lists only shrink: fix the import instead of adding to them.
- **Frame performance.** Code reached from `update`/`ui` must never parse JSON, make blocking system calls (XPC, `SMAppService`), or query SQLite beyond cheap change checks. Cache by change signal (e.g. `Store::layout_index`) or run it on a worker and poll (`work_queue::Pending`, see Background work).

## Build and Test

**Review before you push.** Read your own full diff once more as a reviewer would (naming, leftovers, edge cases, tests, docs), run the commands below, and fix what you find. Then push.

```bash
cargo test
cargo run
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Finder-launchable app: `./tools/bundle.sh`, then open `target/DegenView.app`. Binary: `degenview`.

- CI runs `cargo test --locked`, `cargo fmt --check` and Clippy with `-D warnings` on macOS and Linux
  for every PR. Code used on one platform only is `cfg`-gated, not `allow(dead_code)`.
- Unit tests cover engines, parsing, layout, storage and policy. Manually verify affected UI; outside Pine Studio no complete GUI pass has been done.
- Tests disable Keychain. Launch dev builds with `DEGENVIEW_NO_KEYCHAIN=1` to avoid signing prompts. The MCP credential seal is always off in debug and ignores this variable in release.
- Optional cache override: `DEGENVIEW_CACHE_DIR=<scratch-dir>`.
- Live provider tests need `DEGENVIEW_ALPACA_KEY_ID`, `DEGENVIEW_ALPACA_SECRET_KEY`, or `DEGENVIEW_CMC_API_KEY`.

## Naming

- Snake-case modules; one concern per file.
- **Group modules in directories, never in prefixes.** `model/`, `pine/`, `pine_studio/`, `ui/`,
  `host/`, `service/`, `mcp/`, `sync/`, `workspace/` and `trading/` hold one directory per domain and
  few files at the root (`mod.rs`, plus single-purpose files such as `service/work_queue.rs`,
  `ui/icons`, `workspace/grid`, `pine_studio/workspace.rs`; `store/` keeps one file per single-table store). Use the domain names
  `model/` and `ui/` already use (`alert`, `chart`, `paper`, `portfolio`, `mcp`, ...). A new module goes
  in its domain's directory; a new domain gets a directory. Name the child without the directory's
  name: `model/paper/engine.rs` (`model::paper::engine`), not `model/paper_engine.rs`; `host::paper`,
  not `host::paper_host`. Three or more files sharing a prefix (`alert_*`, `ai_*`, `ta_*`,
  `settings_*`) belong in a directory. `layout_tests.rs` enforces this and the next rule.
- **`mod.rs`, not `foo.rs` + `foo/`.** A module with children or tests is `foo/mod.rs`, tests in
  `foo/tests.rs`. If the parent already has an inline `mod tests`, name end-to-end tests `studio_tests` or
  `integration_tests`. Never leave a `foo.rs` beside a `foo/`.
- **Moving a module.** Rewrite every call site to the new path; no `pub use ... as old_name` alias. A
  moved file's `super::` now points one level lower: use `crate::...` for items in the old parent and
  `pub(in crate::<parent>)` where it was `pub(super)`. Update `include_str!` paths, doc references
  (`docs/`, `AGENTS.md`, `tools/`) and the layout tables in [Modules](../architecture/modules.md).
- Services: grouped by domain under `service/` (`market/`, `keychain/`, `update/`, ...); a provider's module is named after the provider.
- UI: `*_view.rs` or role suffixes such as `*_sheet.rs`, `*_editor.rs`.
- Stores: one file per store in `store/` (a directory once a store has a second file); the `Store` implementation is in `store/mod.rs`.
