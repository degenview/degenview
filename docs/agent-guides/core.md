# Core Agent Guidance

Read for every task, plus the matching topic guides from the root `AGENTS.md` (UI work: [UI style](ui-style.md); persisted data: [Storage and sync](storage.md)).

## Project

DegenView is a GPL-3.0-only Rust market dashboard on `eframe`/`egui`. A small `objc2` AppKit bridge handles native macOS windows, menus, Keychain and accessibility. Keep the coordinator/workspace architecture on other platforms; gate AppKit code with `#[cfg(target_os = "macos")]`.

Store secrets only through `keychain_webhook_secrets::platform` (macOS Keychain, Linux Secret Service, Windows Credential Manager). Never in SQLite, files or logs.

Read [Architecture](../architecture.md) before changing module boundaries or data flow.

## Quality bar

- **Ship finished work.** Code and design are polished and production-ready when you hand them over: no stubs, TODOs, debug output, dead code or placeholder UI, and every state of a screen (empty, loading, error, long text) looks intended. "Works for now" is not done.
- **Decouple, even when it costs a bigger change.** Keep modules independent behind narrow interfaces: pure logic in `model/`, side effects behind hosts and services, no reaching across layers. If the clean design needs a larger change or a refactor of existing code, make it instead of a shortcut that couples more; when it is large, say so first.
- **Clean code is a priority.** Clear names, one concern per file and function, no copy-pasted logic (extract and reuse), comments only for the why, unused code deleted. Match the surrounding style.

## Architecture

- **Modules, not MVVM.** `model/` pure domain logic; `service/` providers/networking; `ui/` egui rendering; `store/` SQLite; `host/*_host.rs` bridges stateful components; `workspace.rs` chart workspaces; `app.rs` coordinates.
- **Immediate-mode UI.** Pass state into views; never add observed view models. Render charts with `egui::Painter`, not a chart library.
- **Background work.** Outside `src/mcp/`, threads and `std::sync::mpsc`, no async runtimes. Shared state `Arc<Mutex<...>>`; keep locks short.
- **Frame performance.** Code reached from `update`/`ui` must never parse JSON, make blocking system calls (XPC, `SMAppService`), or query SQLite beyond cheap change checks. Cache by change signal (e.g. `Store::layout_index`) or run on a thread and poll (e.g. `AlertRuntimeControl::poll_service_status`).

## Build and Test

**Review before you push.** Read your own full diff once more as a reviewer would (naming, leftovers, edge cases, tests, docs), run the commands below, and fix what you find. Then push.

```bash
cargo test
cargo run
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Finder-launchable app: `./tools/bundle.sh`, then open `target/DegenView.app`. Binary: `degenview`.

- CI runs `cargo test --locked` on macOS and Linux for every PR; no Clippy or format checks.
- Unit tests cover engines, parsing, layout, storage and policy. Manually verify affected UI; outside Pine Studio no complete GUI pass has been done.
- Tests disable Keychain. Launch dev builds with `DEGENVIEW_NO_KEYCHAIN=1` to avoid signing prompts. The MCP credential seal is always off in debug and ignores this variable in release.
- Optional cache override: `DEGENVIEW_CACHE_DIR=<scratch-dir>`.
- Live provider tests need `DEGENVIEW_ALPACA_KEY_ID`, `DEGENVIEW_ALPACA_SECRET_KEY`, or `DEGENVIEW_CMC_API_KEY`.

## Naming

- Snake-case modules; one concern per file.
- **Group modules in directories, never in prefixes.** `model/`, `pine/` and `pine_studio/` hold one
  directory per domain and only `mod.rs` at the root (`pine_studio/` also keeps a few single-purpose
  files such as `workspace.rs`). A new module goes in its domain's directory; a new domain gets a
  directory. Name the child without the directory's name: `model/paper/engine.rs`
  (`model::paper::engine`), not `model/paper_engine.rs`. Three or more files sharing a prefix
  (`alert_*`, `ai_*`, `ta_*`) belong in a directory.
- **`mod.rs`, not `foo.rs` + `foo/`.** A module with children or tests is `foo/mod.rs`, tests in
  `foo/tests.rs`. If the parent already has an inline `mod tests`, name end-to-end tests `studio_tests` or
  `integration_tests`. Never leave a `foo.rs` beside a `foo/`.
- **Moving a module.** Rewrite every call site to the new path; no `pub use ... as old_name` alias. A
  moved file's `super::` now points one level lower: use `crate::...` for items in the old parent and
  `pub(in crate::<parent>)` where it was `pub(super)`. Update `include_str!` paths, doc references
  (`docs/`, `AGENTS.md`, `tools/`) and the layout tables in [Modules](../architecture/modules.md).
- Services: `service/*service.rs`, except provider-named modules.
- UI: `*_view.rs` or role suffixes such as `*_sheet.rs`, `*_editor.rs`.
- Stores: `store/*_store.rs`; `Store` implementation in `store/mod.rs`.
