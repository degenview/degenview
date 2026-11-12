# Storage and sync guidance

Read when a change stores, loads or syncs user data. Feature detail: [iCloud Sync](../icloud-sync.md); screen and session state: [Tabs and state](tabs-state.md).

- **SQLite.** All user data through `store::Store` with bundled `rusqlite`. Document tables hold ordered `id/position/payload` JSON rows; `setting` and `window_group` hold keyed values.
- **Schema.** `Store::create_schema` is idempotent. No versioned migrations or legacy readers. Never overwrite a collection with empty data if any row fails to decode.
- **iCloud classification.** Classify every document table and `setting` key in `sync::record::policy`; unclassified data is local-only. Write via `replace_documents` / `set_setting`, which atomically queue sync uploads; never bypass with raw SQL. See [iCloud Sync](../icloud-sync.md).
- **Cross-Mac IDs.** Portfolio and paper-trading IDs use per-Mac counters tagged per database (`sync::record::ids`). Engines must mint IDs below `2^24`. Persist through `save_portfolio_ledger` / `save_paper_trading` or their hosts. `PaperHost` must never fill another Mac's orders until the user takes ownership.
