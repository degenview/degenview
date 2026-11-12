# MCP guidance

## Threads and authorization

- Only `src/mcp/` may use async: one current-thread Tokio runtime on the `mcp-server` thread for
  rmcp/axum. Keep it egui-free; `host/mcp/mod.rs` bridges stateful workspace access.
- Tools validate and enqueue `mcp::bridge::Request`; `DegenViewApp::tick_mcp` runs it via workspace
  methods on the app thread. Tool/HTTP threads never lock `TabSlot`/`ChartWorkspace` or write SQLite.
- Authorize each call from fresh `McpShared` state. New tool: add a `Permission`, map it in
  `tools::required_permission`, test direct-call denial. Unmapped routed tools never run (`call_tool`
  fails closed; `every_routed_tool_has_a_permission_and_an_unmapped_name_is_not_run` fails the
  build).
- Persist before publishing shared state. Store/log only SHA-256 token verifiers, never tokens.
  `McpShared::authenticate` carries a match across its lock gap as the client id, re-checking the
  verifier; never a list index.
- Settings holds the store lock: its MCP UI returns `McpAction`s for `McpHost::queue`; never call
  `McpHost::apply` inside the Settings closure.

## Sealed trust state

- `mcp.settings`, `mcp.permissions` and every `mcp_client` row are HMAC-sealed under a Keychain key
  (`mcp::transport::seal`, `keychain::mcp_seal`); a bare verifier would let anyone who can write the SQLite file
  add a credential.
- Write them only via `McpHost::persist` (`Store::save_mcp_state`: all three plus `mcp.seal` in one
  savepoint, rewritten from memory, never patched). Never `set_mcp_settings`,
  `set_mcp_global_permissions` or `replace_mcp_clients` for a trust change. `last_used_ms` is
  unsealed on purpose, so the usage save may use `replace_mcp_clients`.
- New access-deciding field: add to `mcp::transport::seal::Sealed` plus a case in `every_trust_field_is_covered`.
- Sealing is off in tests, every debug build (new code identity per rebuild = Keychain prompt every
  launch), and `--release --features dev-unsealed-mcp` builds started with `DEGENVIEW_NO_KEYCHAIN`;
  other release builds ignore that variable, never rely on it there. Test with
  `McpHost::load_with_keys` and a shared `MemoryBackend`. See
  [Sealed credentials](../mcp-implementation.md#sealed-credentials).

## Cancellation, audit, limits

- A dropped bridge call cancels its queued `Request` (`CancelOnDrop`); keep it in
  `BridgeClient::call`. rmcp keeps a handler alive after the client leaves, so keep `call_tool`'s
  race against `context.ct`; never spawn work outliving the tool's future without a drop guard.
  Stateless `notifications/cancelled` arrives as a separate POST: `call_tool` registers calls in
  `McpShared::calls()`, `cancel::honour_cancellations` cancels them. That layer sits outside the
  concurrency cap, inside the bearer check.
- `AuditGuard` in `call_tool` (`mcp/audit.rs`) records every `tools/call`; any new way to end a call
  goes through it. Rows hold only what `audit.rs` allows (id-shaped ids, a code), never an argument
  value or `McpError::message()`. `McpHost::tick` writes the log, not request threads; Settings and
  the activity sheet read `McpHost::activity()`, never SQLite. Filtering/paging: `model::mcp::activity`
  (pure, tested); a new outcome or row kind needs a `McpActivityStatus` there. `mcp_audit` is
  local-only in `sync::record::policy`.
- Rate limits: `mcp/transport/limits.rs`, applied in `require_bearer`. Bad tokens may get 429; a valid token
  is never refused for others' failures. `transport::serve` (not `axum::serve`) accepts connections
  so header, time and connection limits hold; add limits there.

## Confirmation

- Classify each new mutating `McpCommand` in `mcp::confirm::gate` (exhaustive match). If it asks:
  describe it in `App::confirmation_summary` / `host::mcp::confirmation` from what the app knows of
  the target, never agent text (missing target: own error, unasked); add its tool to
  `confirm::GATED_TOOLS`; say so in the tool description.
- The gate runs on the app thread before dispatch, never waits: it answers `CONFIRMATION_REQUIRED`
  and the agent repeats the call. An approval is consumed by its call; never widen it beyond the
  exact command and client. An "all" command records what it resolved to in `Summary::with_scope`,
  so approval never covers later additions. Only `Summary::client_line` (the action) may reach a
  client's response; target text can name things the credential may not read.
- Approval is decided by `confirm::needs_approval(shared, client_id, command)`, never `gate` alone.
  `remote_allowed` credential: asked about every change (`Gate::Ask`, `Gate::AskRemote`), "ask before
  high-impact requests" setting ignored. Local: `Gate::Ask` only, never with the setting off. An
  `AskRemote` command also needs a `tool_of` name and `confirm::remote_summary`, else it is refused,
  not run unasked. Turning the setting off drains only local questions
  (`ConfirmationBook::drain_where`).

## Tools

- Price alerts: plan against `AlertHost` in `host/mcp/mod.rs`, enqueue `AlertRuntimeCommand`; never
  write alert tables or reimplement `PriceAlertEditorState`. The snapshot is up to 1 s old and
  shared per frame, so commands carry an `AlertPrecondition` (`Create`: id free, below the cap, no
  identical alert; `Unchanged`: alert still matches the planned base) checked by
  `AlertEngine::apply`. Refusals land in `rejected_commands` and surface as `CONFLICT` via
  `McpCommand::AlertCommandOutcome`. Never use snapshot `revision` as a precondition (moves on every
  quote and delivery). An unqueueable save is an error, never "queued".
- Read-only script tools: `mcp/pine.rs` with `ScriptStore` and compiler on a worker; chart-private
  Pine state via `ChartWorkspace::pine_instance_views` on the app thread. Never invent reports or
  alert history.
- Scripts on charts: `ChartWorkspace::{add_pine_instance_to,
  change_pine_instance,remove_pine_instance_from}` (take the undo step); validate scripts and inputs
  with `mcp::pine` before queuing; apply only saved text, never the draft.
- Pine edits: revision-checked `mcp::pine::{update_draft,save_script}` on the app thread, then
  `PineStudio::adopt_external_text`. Never bypass the live editor revision or write script files
  from a worker. Saves compare the file with what the editor last read
  (`PineStudio::open_disk_state`, `ScriptStore::save_if_unchanged`) and are refused during an
  unresolved external-change conflict; never `ScriptStore::save` from MCP.
- Drawings: plan from `ChartWorkspace::chart_data` with `mcp::chart::data`, commit via
  `apply_planned_drawings` (pointer's path, one undo step); tool threads never read `Chart.drawings`
  or `Chart.candles`. Validate in `mcp::chart::drawings` before queueing; never edit a locked drawing.
- `degenview_get_candles` returns the market's own candles, not constructed bars. Indicator readings
  use the drawn bars (`ChartWorkspace::constructed_bars` for Heikin Ashi, Renko etc.), never raw
  candles.
- Search/quotes (`mcp/market/tools.rs`) run on tool threads, no bridge: `McpShared::market_search`;
  a `QuoteLease` on the app's quote feed (`attach_quotes`), released on drop. Watchlist tools use the
  bridge and `WatchlistStore::edit_list`; never write watchlist rows from a tool thread. Second
  permissions (`get_watchlist` with `include_quotes`): `McpShared::allows`.
- Live trading is unreachable: nothing under `src/mcp/` may mention `degenview_brokers` or
  `host::broker` (`host::broker::tests` fails the build). A tool reaching a real broker needs its
  own design, permission and confirmation, not a new arm here. See [Live trading](../broker-trading.md).
- Paper writes (`mcp/paper/tools.rs`, `host::mcp::place_paper_order` and friends): only via
  `PaperHost::{submit,modify,cancel,close_position}`, `tradable_account` checked first (another
  Mac's account is never changed over MCP), each new write behind its own off-by-default permission.
  A paper `request_id` is bound to its order (another order under it: `CONFLICT`).
- Write tools never return data behind a read grant the caller lacks: paper writes drop the account
  without `paper.read` (re-checked after the write). Anything waited on is re-checked afterwards for
  every permission covering the final payload.

## Remote access

- Non-loopback: `BindMode::Interface(ip)`, one named address, TLS only, never `0.0.0.0` or `::`
  (`BindMode::interface` rejects them; a stored one fails the load). Sole gate: `Listen::check` in
  `McpServer::start_listening`, refusing before a socket opens any address `BindMode::interface`
  would refuse (the variant can be built directly), a missing certificate, or one for another
  address. Never add a bind path around it. `bind` is sealed but written only when not loopback, so
  old seals keep verifying.
- Certificate: `mcp::transport::tls` (`mcp-tls.pem` in the data directory; owner-only on Unix, DPAPI on Windows
  via `mcp::transport::tls_protect`). Only through `AtRest`: repaired on load, written via a fresh temp file,
  held to naming its recorded address, created under the `mcp-tls.lock` lock so processes share one
  identity; new steps go inside `load_or_create_locked`. `Host` list: `BindMode::allowed_hosts`, not
  the port. See "Remote access" in [implementation](../mcp-implementation.md).
- Origin: `McpShared::authenticate_from(token, peer)`, from the socket peer `transport::serve_with`
  records as `PeerAddr`, never a header. Credentials are local-only unless `McpClient::remote_allowed`
  (then anywhere, or only `allowed_cidrs` if set). Local = `Peer.local`: loopback or the address the
  connection arrived on (this Mac via the bound LAN address counts); use `Peer::of_connection`, never
  `is_loopback()`. A refused address looks like an unknown token (401, failed-auth bucket), records
  no use. No peer = remote. Never add a second way to authenticate.
- `McpClient::effective(global, remote_ceiling)` caps a `remote_allowed` client by
  `McpSettings::remote_ceiling` (default: every read-only permission), wherever it calls from. These
  client, settings and ceiling fields are sealed and omitted from the stored form while default, so
  old rows and seals read back unchanged.
- A queued `Request` carries its `Origin` (peer, `remote`, from `ClientIdentity`): pass the identity
  to `BridgeClient::call`, never a bare client id. `Request::run` re-checks the address
  (`accepts_origin`) and gives the origin to the gate (`needs_approval(.., remote_origin, ..)`);
  never decide "remote" at execution from the credential's current state alone.
- Remote failed sign-ins hit the address's own bucket first (`RateLimiter::failed_auth(peer, now)`,
  bounded table), then the shared one; one `AuthRefused` row per address per minute. Audit rows
  carry `peer` (empty for this machine), never a token or request content.
- Security card "Remote connections" (`ui/settings/mcp.rs`) saves nothing until `SetBind` is sent
  with an address `parse_interface` accepted. Fingerprint from `McpHost::tls_fingerprint`, never
  computed in the page. `detect_local_address` is a system call: on click only, never per frame.

## Verifying

`127.0.0.1:<port>/mcp` with seeded `mcp.settings`/`mcp_client` rows, throwaway `HOME`,
`DEGENVIEW_NO_KEYCHAIN=1` (seeded rows only load unsealed: debug build or
`cargo build --release --features dev-unsealed-mcp`), `Authorization: Bearer ...`.

See [implementation](../mcp-implementation.md) and [remaining work](../mcp-remaining-work.md).
