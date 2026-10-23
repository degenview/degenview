# Core agent guidance

Read this file for every task. Load the matching topic guides listed in the root `AGENTS.md` too.

## Project

DegenView is a GPL-3.0-only market dashboard written in Rust with `eframe`/`egui`. A small `objc2`
AppKit boundary handles native macOS windows, menus, Keychain, and accessibility. Non-macOS builds
keep the coordinator/workspace model and gate AppKit code with `#[cfg(target_os = "macos")]`.

See [the architecture map](../architecture.md) before changing module boundaries or data flow.

## Architecture rules

- Use modules, not MVVM: `model/` is pure domain logic, `service/` owns providers and network
  boundaries, `ui/` contains egui renderers, `store/` owns SQLite, `host/*_host.rs` files bridge
  stateful engines/stores/views, `workspace.rs` owns one chart workspace, and `app.rs` coordinates.
- egui is immediate-mode. Pass state into views; do not add observed view models. Charts use
  `egui::Painter`, not a chart library.
- Docked panel edges use egui's panel separator only (`theme::border()`). Do not add `Frame::stroke`
  or hand-painted edge lines to a docked `egui::Panel`: they stack with the separator and make
  borders look thicker where panels meet. Use `show_separator_line(false)` only when the panel paints
  a deliberate replacement, as the replay bar's tinted line does.
- Pop-up menus are built from `kit::menu_row`, `menu_item`, `menu_heading`, `menu_separator` and
  `menu_submenu`, inside `kit::menu_popup` (or `kit::context_popup` for a right-click) with
  `kit::menu_scope` first in the body. Do not put bare `ui.button`, `radio`, `checkbox` or
  `menu_button` in a menu: egui frames those itself, so they miss the menu style. Menus stay open
  on a click, so a row that ends the menu calls `ui.close()` and a toggle does not.
- Outside `src/mcp/`, use background threads and `std::sync::mpsc`, not an async runtime. Shared
  state is `Arc<Mutex<...>>`; keep store locks short.
- User data lives in bundled-rusqlite SQLite through `store::Store`. Document tables are ordered
  `id/position/payload` JSON rows; `setting` and `window_group` are keyed values.
- `Store::create_schema` is one idempotent schema; there are no versioned migrations or legacy
  readers. Never replace a collection with empty data when any row is undecodable.
- Every document table and `setting` key must be classified for iCloud Sync in
  `sync::policy` (a test fails otherwise); unclassified means local-only. Writes go through
  `replace_documents` / `set_setting`, which queue the upload in the same transaction when sync is
  on, so do not write those tables with raw SQL. See [iCloud Sync](../icloud-sync.md).
- Portfolio and paper-trading ids are per-Mac counters that sync under a per-database tag
  (`sync::ids`). Engines and ledgers must keep minting from ids below 2^24 only, writes go through
  `save_portfolio_ledger` / `save_paper_trading` (or the hosts) so they are tracked, and a paper
  account another Mac runs must never be filled here (`PaperHost` skips it until the user acts on
  it).

## Build and test

```bash
cargo test
cargo run
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Build a Finder-launchable app with `./tools/bundle.sh`, then open `target/DegenView.app`. The binary
is `degenview`.

CI runs `cargo test --locked` on macOS and Linux for every pull request, but not clippy or fmt.
Unit tests cover pure engines, parsing, layout, storage, and policy. Manually verify affected UI;
outside the Script Manager, the app has not had a complete manual GUI pass.

Tests disable Keychain. For development launches set `DEGENVIEW_NO_KEYCHAIN=1` to avoid signing-based
Keychain prompts, and optionally `DEGENVIEW_CACHE_DIR=<scratch-dir>`. Live provider checks require
their `DEGENVIEW_ALPACA_KEY_ID`, `DEGENVIEW_ALPACA_SECRET_KEY`, or `DEGENVIEW_CMC_API_KEY` variables.

## Naming

- Use snake-case modules and one concern per file.
- Services are `*service.rs` under `service/`, except provider-named modules.
- UI files use `*_view.rs` or a role suffix such as `*_sheet.rs` or `*_editor.rs`.
- Stores use `*_store.rs` under `store/`, with the `Store` implementation in `store/mod.rs`.
