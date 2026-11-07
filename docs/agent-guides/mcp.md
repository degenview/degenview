# MCP guidance

- `src/mcp/` alone may use async: one current-thread Tokio runtime on the `mcp-server` thread for
  rmcp/axum. Keep it egui-free; `host/mcp_host.rs` bridges stateful workspace access.
- Tools validate and enqueue `mcp::bridge::Request`; `DegenViewApp::tick_mcp` executes through
  workspace methods on the app thread. Tool/HTTP threads never lock `TabSlot`/`ChartWorkspace` or
  write SQLite.
- Authorization is per call from fresh `McpShared` state. Add a `Permission`, map it in
  `tools::required_permission`, and test direct-call denial. A routed tool with no mapping is never
  run (`call_tool` fails closed; `every_routed_tool_has_a_permission_and_an_unmapped_name_is_not_run`
  fails the build). Persist before publishing shared state. Store/log only SHA-256 token verifiers,
  never tokens. `McpShared::authenticate` carries a match across its lock gap as the client id and
  re-checks the verifier; never carry a list index.
- The trust state (`mcp.settings`, `mcp.permissions`, every `mcp_client` row) is sealed with an
  HMAC under a Keychain key (`mcp::seal`, `keychain_mcp_seal`), because a SHA-256 verifier alone
  lets anyone who can write the SQLite file add a credential. Write all of it with
  `McpHost::persist` (`Store::save_mcp_state`: the three values and `mcp.seal` in one savepoint,
  rewritten from memory, never patched); never call `set_mcp_settings`, `set_mcp_global_permissions`
  or `replace_mcp_clients` for a trust change. `last_used_ms` is outside the seal on purpose, so
  the usage save may use `replace_mcp_clients`. A new field that decides access goes into
  `mcp::seal::Sealed`, with a case in `every_trust_field_is_covered`. Sealing is off in tests and
  in debug (or `--features dev-unsealed-mcp`) builds started with `DEGENVIEW_NO_KEYCHAIN`; a
  release build ignores that variable, so never rely on it there; use `McpHost::load_with_keys` with a shared
  `MemoryBackend` to test it. See [Sealed credentials](../mcp-implementation.md#sealed-credentials).
- A bridge call that is dropped (client closed the stream, request cancelled) cancels its queued
  `Request` (`CancelOnDrop`); keep it that way when changing `BridgeClient::call`. rmcp does not
  drop a handler when the client leaves, so `call_tool` races the tool against `context.ct`: keep
  that race, and never spawn work from a tool that outlives its future without a drop guard.
  A `notifications/cancelled` is a separate POST in stateless mode: `call_tool` registers each call
  in `McpShared::calls()` and `cancel::honour_cancellations` cancels it. Keep that layer outside the
  concurrency cap and inside the bearer check.
- Every `tools/call` is recorded by `AuditGuard` in `call_tool` (`mcp/audit.rs`). A new way for a
  call to end must end through that guard, and a row may only hold what `audit.rs` lets in: ids that
  look like ids, a code, never an argument value or `McpError::message()`. The log is written by
  `McpHost::tick`, not from request threads; the Settings page and the activity sheet read
  `McpHost::activity()`, never SQLite. The sheet's filtering and paging live in
  `model::mcp_activity` (pure, tested); a new outcome or row kind needs a `McpActivityStatus`
  there so it can be filtered. `mcp_audit` is local-only in `sync::policy`.
- A new mutating `McpCommand` must be classified in `mcp::confirm::gate` (an exhaustive match: it
  fails to compile until it is). If it asks, also describe it in
  `App::confirmation_summary` / `host::mcp_confirmation` (from what the app knows of the target,
  never from agent text; a target that does not exist is refused with its own error, unasked), add
  its tool to `confirm::GATED_TOOLS` and say so in the tool's description. The gate runs on the app
  thread before dispatch and must never wait for the user: it answers `CONFIRMATION_REQUIRED` and
  the agent repeats the call. Approvals are used up by the call that runs them; never widen one
  beyond the exact command and client. A command that reaches "everything of a kind" (an "all")
  must put what it resolved to in `Summary::with_scope`, so an approval cannot cover things that
  appeared after it. Only `Summary::client_line` (the action) may go into a client's response:
  the target text can name things the credential may not read.
- Rate limits live in `mcp/limits.rs` and are applied in `require_bearer`. Bad tokens may be
  refused with 429; a valid token must never be refused because of other callers' failures.
  Connections are accepted by `transport::serve` (not `axum::serve`) so header, time and
  connection limits hold; keep new limits there.
- Price-alert tools plan against `AlertHost` in `host/mcp_host.rs`, then enqueue
  `AlertRuntimeCommand`; they never write alert tables or reimplement `PriceAlertEditorState`.
  A plan is made from a snapshot that is up to a second old and shared by every request of the
  frame, so the command carries an `AlertPrecondition` (`Create`: id free, below the cap, no
  identical alert; `Unchanged`: the alert still matches the planned base) that `AlertEngine::apply`
  checks. A refused command is recorded in `rejected_commands` and surfaces as `CONFLICT` through
  `McpCommand::AlertCommandOutcome`. Do not use the snapshot `revision` as a precondition: it moves
  on every quote and delivery. A save that cannot be queued is an error, never "queued".
- Read-only script tools use `mcp/pine.rs` with `ScriptStore` and compiler on a worker. Chart-private
  Pine state goes through `ChartWorkspace::pine_instance_views` on the app thread. Do not invent
  reports or alert history.
- Applying/changing/removing a script on a chart goes through `ChartWorkspace::{add_pine_instance_to,
  change_pine_instance,remove_pine_instance_from}`, which take the undo step; validate scripts and
  inputs with `mcp::pine` before queuing, and apply only the saved text, never the draft.
- Pine edits use revision-checked `mcp::pine::{update_draft,save_script}` on the app thread, followed
  by `PineStudio::adopt_external_text`. Never bypass the live editor revision or write script
  files from a worker. A save compares the file with what the open editor last read
  (`PineStudio::open_disk_state`, `ScriptStore::save_if_unchanged`) and refuses while the editor has
  an unresolved external-change conflict; never call `ScriptStore::save` from the MCP path.
- Drawings from MCP are planned from `ChartWorkspace::chart_data` with `mcp::chart_data` and
  committed through `apply_planned_drawings` (the pointer's path, one undo step); tool threads never
  read `Chart.drawings` or `Chart.candles`. Candles returned are the market's own, not constructed
  bars. Validate arguments in `mcp::drawings` before queueing, and never edit a locked drawing.
- Symbol search and quotes (`mcp/market_tools.rs`) run on tool threads without the bridge: search
  through `McpShared::market_search`, quotes through a `QuoteLease` on the app's quote feed
  (`attach_quotes`), released on drop. Watchlist tools go through the bridge and
  `WatchlistStore::edit_list`; never write watchlist rows from a tool thread. A tool that needs a
  second permission (`get_watchlist` with `include_quotes`) checks it with `McpShared::allows`.
- Paper-trading writes (`mcp/paper_tools.rs`, `mcp_host::place_paper_order` and friends) go through
  `PaperHost::{submit,modify,cancel,close_position}` only, always check
  `tradable_account` first (an account another Mac runs is never changed over MCP), and keep every
  new write behind its own off-by-default permission. No tool may reach a real broker.
- A write tool never returns data that sits behind a read grant the caller lacks: paper writes
  drop the account unless the credential also holds `paper.read` (re-checked after the write).
  Anything a tool waits on is re-checked after the wait, for every permission covering the final
  payload.
- Indicator readings use the bars the chart draws (`ChartWorkspace::constructed_bars` for Heikin
  Ashi, Renko and the other built types), never the raw candles; `degenview_get_candles` stays the
  market's own. A paper `request_id` is bound to its order (another order under it is `CONFLICT`).
- Listening beyond loopback is `BindMode::Interface(ip)`: one named address (never `0.0.0.0` or
  `::`, which `BindMode::interface` rejects and a stored one fails the load), TLS only. The only
  gate is `Listen::check` in `McpServer::start_listening`: an address `BindMode::interface` would
  refuse (the variant can be built directly), no certificate, or one made for another address, is
  refused before a socket opens. Do not add a path that binds without it. `bind` is
  sealed, but only written when it is not loopback, so existing seals keep verifying. The
  certificate is `mcp::tls` (`mcp-tls.pem` in the data directory, owner-only: repaired on load, written
  through a fresh temporary file, held to naming the address it records, and made under the
  `mcp-tls.lock` lock so concurrent processes share one identity; keep any new step of that flow inside
  `load_or_create_locked`); the `Host` list comes
  from `BindMode::allowed_hosts`, not the port. See "Remote access" in
  [implementation](../mcp-implementation.md).
- Where a request may come from is decided in `McpShared::authenticate_from(token, peer)`, from the
  socket peer `transport::serve_with` records as `PeerAddr`, never from a header. A credential works
  from this machine unless `McpClient::remote_allowed`. "This machine" is `Peer.local`: loopback or the
  address the connection arrived on (a client on this Mac using the LAN address the server is bound to
  is local; use `Peer::of_connection`, never `is_loopback()`, to decide it). Otherwise from anywhere, or only from
  `allowed_cidrs` if it has any. A refused address looks like an unknown token (401, and it draws on
  the failed-auth bucket) and records no use. No peer means remote. Do not add a second way to
  authenticate. `McpClient::effective(global, remote_ceiling)` also caps a `remote_allowed` client by
  `McpSettings::remote_ceiling` (default: every read-only permission), wherever it calls from. The new
  client, settings and ceiling fields are sealed, and left out of the stored form while they are the
  default, so old rows and seals read back unchanged.
- Settings holds the store lock, so its MCP UI returns `McpAction`s for `McpHost::queue`; never call
  `McpHost::apply` inside the Settings closure.
- Verify against `127.0.0.1:<port>/mcp` with seeded `mcp.settings`/`mcp_client` rows, a throwaway
  `HOME`, `DEGENVIEW_NO_KEYCHAIN=1` (seeded rows are only accepted unsealed, which needs a debug
  build or `cargo build --release --features dev-unsealed-mcp`), and `Authorization: Bearer ...`.

See [implementation](../mcp-implementation.md) and [remaining work](../mcp-remaining-work.md).
