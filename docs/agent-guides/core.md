# Core Agent Guidance

Read this file for every task, plus relevant topic guides from the root `AGENTS.md`.

## Project

DegenView is a GPL-3.0-only Rust market dashboard using `eframe`/`egui`. A small `objc2` AppKit bridge handles native macOS windows, menus, Keychain, and accessibility. Preserve the coordinator/workspace architecture on other platforms; gate AppKit code with `#[cfg(target_os = "macos")]`.

Store secrets only through `keychain_webhook_secrets::platform` (macOS Keychain, Linux Secret Service, Windows Credential Manager). Never put secrets in SQLite, files, or logs.

Read [Architecture](../architecture.md) before changing module boundaries or data flow.

## Architecture

- **Modules, not MVVM.** `model/` contains pure domain logic; `service/` owns providers/networking; `ui/` renders egui; `store/` owns SQLite; `host/*_host.rs` bridges stateful components; `workspace.rs` owns chart workspaces; `app.rs` coordinates.
- **Immediate-mode UI.** Pass state into views; never introduce observed view models. Render charts with `egui::Painter`, not a chart library.
- **Docked panels.** Use egui's separator (`theme::border()`) exclusively. Never add `Frame::stroke` or painted edge lines. Use `show_separator_line(false)` only when deliberately replacing the separator.
- **Background work.** Outside `src/mcp/`, use threads and `std::sync::mpsc`, not async runtimes. Shared state uses `Arc<Mutex<...>>`; keep locks short.
- **Frame performance.** Code reached from `update`/`ui` must never parse JSON, perform blocking system calls (XPC, `SMAppService`), or query SQLite beyond cheap change checks. Cache by change signal (e.g. `Store::layout_index`) or run work on a thread and poll results (e.g. `AlertRuntimeControl::poll_service_status`).

## UI Conventions

- **Menus.** Use `kit::menu_row`, `menu_item`, `menu_heading`, `menu_separator`, and `menu_submenu` inside `kit::menu_popup` or `kit::context_popup`, calling `kit::menu_scope` first. Never use bare egui buttons, radios, checkboxes, or menu buttons. Call `ui.close()` for actions that dismiss the menu; toggles keep it open.
- **Menu sizing.** Fit menus to their longest row, never wider. Use `kit::fitted_menu` / `fitted_menu_rows` for button menus, `fitted_context_menu` for right-clicks, or `menu_fit` / `menu_fit_rows` inside popups. Helpers measure dynamic menus only while open. For fixed-width menus, test against `kit::menu_width_for_rows` (see `menu_widths_fit_their_rows` in `watchlist_sidebar.rs`). Never guess widths with `set_min_width` or `set_width`. Visually check that the longest label ends one row inset from the menu edge.
- **Buttons are neutral, never blue.** No blue accent in button fills, borders, labels, or outlines. Use `kit::sheet_button` (`prominent`), `kit::push_button`, footer Primary, or `theme::card_fill` / `card_border` / `card_highlight` with `text_primary`. Large sheet actions use `kit::large_prominent_button`; destructive actions use `kit::destructive_sheet_button`. Accent colors belong to data and state, not controls. Compare new buttons with neighboring sheets. Existing accent-colored buttons have not all been audited.
- **Links are gray.** Use underlined `theme::text_secondary()` for hyperlinks and link-like text, never blue accents. Do not introduce new decorative accent usage or change existing selection/focus colors without being asked.
- **Flat styling.** New UI uses faint fills and hairline borders, without gradients, highlights, shadows, or icon backgrounds. See [Flat Style](ui-style.md).
- **Dialogs.** Use `kit::confirm`, `kit::alert`, or `kit::notice` through `ui::dialog::show`; never custom modal frames or stacked full-width buttons. Standard layout: tinted title badge, message, close button, and bottom-right Cancel/action buttons. Use `Dialog::destructive` for destructive actions (red, Cancel focused), `cautious` when Enter must not approve, `body` for additional content, and `leading` for a third left-side button. Tint only the badge, never blue. Large forms belong in `ui::sheet`. Verify in `gallery_of_the_dialogs`.

## Storage and Sync

- **SQLite.** All user data goes through `store::Store` with bundled `rusqlite`. Document tables contain ordered `id/position/payload` JSON rows; `setting` and `window_group` contain keyed values.
- **Schema.** `Store::create_schema` is idempotent. No versioned migrations or legacy readers. Never overwrite a collection with empty data if any row fails to decode.
- **iCloud classification.** Classify every document table and `setting` key in `sync::policy`; unclassified data is local-only. Use `replace_documents` / `set_setting`, which atomically queue sync uploads. Never bypass them with raw SQL. See [iCloud Sync](../icloud-sync.md).
- **Cross-Mac IDs.** Portfolio and paper-trading IDs use per-Mac counters tagged per database (`sync::ids`). Engines must mint IDs below `2^24`. Persist through `save_portfolio_ledger` / `save_paper_trading` or their hosts. `PaperHost` must never fill orders belonging to another Mac until the user takes ownership.

## Build and Test

```bash
cargo test
cargo run
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Build a Finder-launchable app with `./tools/bundle.sh`, then open `target/DegenView.app`. Binary: `degenview`.

- CI runs `cargo test --locked` on macOS and Linux for every PR, but not Clippy or formatting checks.
- Unit tests cover engines, parsing, layout, storage, and policy. Manually verify affected UI; outside Pine Studio, a complete GUI pass has not been performed.
- Tests disable Keychain. For development launches, use `DEGENVIEW_NO_KEYCHAIN=1` to avoid signing prompts. The MCP credential seal is always disabled in debug and ignores this variable in release.
- Optional cache override: `DEGENVIEW_CACHE_DIR=<scratch-dir>`.
- Live provider tests require `DEGENVIEW_ALPACA_KEY_ID`, `DEGENVIEW_ALPACA_SECRET_KEY`, or `DEGENVIEW_CMC_API_KEY`.

## Naming

- Snake-case modules; one concern per file.
- **Group modules in directories, never in prefixes.** `model/`, `pine/` and `pine_studio/` hold one
  directory per domain and nothing but `mod.rs` at the root (`pine_studio/` also keeps a few
  single-purpose files such as `workspace.rs`). A new module goes into the directory of its domain, and a
  new domain gets a directory. Name the child without the directory's name: `model/paper/engine.rs`
  (`model::paper::engine`), not `model/paper_engine.rs`. When three or more files share a prefix
  (`alert_*`, `ai_*`, `ta_*`), they belong in a directory.
- **`mod.rs`, not `foo.rs` + `foo/`.** A module with children or tests is `foo/mod.rs`, with tests in
  `foo/tests.rs`. If the parent already has an inline `mod tests`, name end-to-end tests `studio_tests` or
  `integration_tests`. Do not leave a `foo.rs` next to a `foo/`.
- **Moving a module.** Rewrite every call site to the new path; do not keep the old name alive with a
  `pub use ... as old_name` alias. A moved file's `super::` now points one level lower: use `crate::...`
  for items in the old parent, and `pub(in crate::<parent>)` where it was `pub(super)`. Update
  `include_str!` paths, doc references (`docs/`, `AGENTS.md`, `tools/`) and the layout tables in
  [Modules](../architecture/modules.md).
- Services: `service/*service.rs`, except provider-named modules.
- UI: `*_view.rs` or role suffixes such as `*_sheet.rs`, `*_editor.rs`.
- Stores: `store/*_store.rs`; `Store` implementation in `store/mod.rs`.