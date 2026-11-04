# MCP server production-readiness review

Date: 2026-11-04

Scope: `src/mcp/`, `src/host/mcp_host.rs`, the MCP execution paths in `src/app.rs`, MCP persistence and permissions, Settings integration, and the alert/Pine APIs those paths rely on.

## Verdict

The MCP server has a strong security-oriented foundation, but it is **not production-ready yet**. I found four release-blocking defect classes:

1. authentication can associate a verified token with the wrong client during a concurrent client-list replacement;
2. saving Pine source through MCP can overwrite an external file edit that the open editor has not reconciled;
3. price-alert changes are planned against a lagging snapshot and queued without revision enforcement, allowing duplicates, lost updates, resurrection after deletion, and false success on create;
4. dropping or cancelling a bridge call does not cancel its queued request, so a destructive operation can still execute after its caller has abandoned it.

The implementation should not be promoted as production-ready until those issues are fixed and covered by deterministic race/negative-path tests. Audit logging and confirmation for high-impact operations are also release requirements, not optional polish, given that the server can overwrite saved source and permanently delete alerts.

## Status

Updated after validating every finding against the code on `main` (d782f68) and fixing them in one
pull request: [#318](https://github.com/degenview/degenview/pull/318). `cargo test --lib` (5,395 tests, corpus excluded),
`cargo fmt --check` and `cargo clippy --all-targets -D warnings` pass. Each new regression test for the bridge and alert fixes was
run once with its fix removed and failed; the others were not
checked that way. Nothing here was exercised with a real MCP client, against the background agent
draining alert commands, or in a real Pine Studio window. CI results are on the pull request, not
recorded here. The sections below describe the code as reviewed.

| ID | Status | Note |
| --- | --- | --- |
| P1 Authentication race | Fixed | The match is carried across the lock gap as the client id and the verifier is checked again under the write lock; a revoked or replaced credential gets no identity. Deterministic test through a between-locks seam (`authenticate_with`). Failed authentication still takes only the read lock. |
| P1 Pine save overwrites an external edit | Fixed, narrower | The save goes through `ScriptStore::save_if_unchanged` against what the open editor last read, refuses while the editor has an unresolved conflict, and `ChangedOnDisk` is `CONFLICT` (the editor is asked to look at the file at once, as its own Save does). Tests: clean editor, dirty editor, unresolved conflict, an editor that has seen the file. The race "immediately before the write" is not reproducible from a test; the comparison is made by `save_if_unchanged` itself. **Not fixed:** a script with no open editor but a draft can still overwrite a later outside edit, because a draft records no base (`based_on_revision_id` is always `None`). This case was not in the review. |
| P1 Alert mutations not atomic | Fixed, differently | The review's global `expected_revision` cannot work: the snapshot revision moves on every trigger, delivery and health write, so nearly every command would be refused. Commands now carry an `AlertPrecondition` that `AlertEngine::apply` checks where the data lives: `Create` (id free, below the cap, no identical alert) and `Unchanged` (the alert exists and still matches the definition the edit was planned from; the runtime's own fields are not compared). A refused command is consumed and recorded in `rejected_commands` (persisted, local only), and the MCP server polls `AlertCommandOutcome` and answers `CONFLICT` or `RESOURCE_LIMIT`. Pause/resume carry no precondition: `set_state` on a missing alert already does nothing, and an edit planned from the other state is refused by its own `Unchanged`. Tests apply several requests planned against one snapshot, drained once: identical creates, a burst at the cap, disjoint edits, edit and delete in both orders, and a reload from the store. `expected_revision` was never read and is removed; queued rows that still carry it decode. |
| P1 Alert create reports success when enqueue fails | Fixed | `commit_price_alert_checked` returns the queued command id or the error; nothing else runs after a failed save (no authorization request, no service registration). MCP create answers `APP_UNAVAILABLE`. The sheets keep ignoring a failed save as before, minus the side effects. |
| P1 Abandoned bridge calls stay armed | Fixed | `CancelOnDrop` moves the request from pending to cancelled when the call is dropped, and `Request::run` also refuses a request whose reply channel is closed. Tests abort a call before the inbox is drained, behind other requests, and after the app started it (its outcome stands). **Not done:** passing rmcp's own request cancellation token down, and a real-client check that closing the stream drops the call. |
| P2 Permission enforcement fails open for unmapped tools | Fixed, partly | `call_tool` refuses any tool with no mapping; a catalog test fails when a routed tool has none. Deriving the bridge permission from the command instead of each handler is still open. |
| P2 Concurrency limit after authentication | Fixed, partly | A cap of 64 requests now sits outside the bearer check. Failed-authentication and per-client rate limits and explicit header limits are still open. The JSON nesting point does not hold: serde_json already stops at a depth of 128. |
| P2 Pine compile bursts on the UI thread | Open | Needs preparing the compile on the Pine workers and a short revision recheck and commit on the app thread. Not changed here. |
| P2 Drawing id exhaustion | Fixed | The whole id range is checked before planning, so a stored drawing with the highest id gives `RESOURCE_LIMIT` instead of a panic (debug) or a wrap (release). Boundary tests at `u64::MAX - 1` and `u64::MAX`. `src/workspace/chart.rs` has the same unchecked `max + 1` for the GUI's own ids; left alone. |
| P3 Fractional seconds discarded | Fixed | One to nine digits are read and cut to milliseconds (never rounded up); an empty fraction or more than nine digits is refused. |
| P3 Listener status races startup failure | Fixed | The server thread reports once the runtime owns the listener and sets `Running` itself; the caller waits (10 s) and returns the error; a panic on the server thread sets `Failed`. Runtime-creation and listener-conversion failures are not exercised by a test. |
| P3 Swallowed errors | Fixed | A failed save of "last used" times is retried on the next tick. A compile that could not run after a Pine edit gives `compile_warning` instead of `compile: null`. The compile tool names `degenview_add_pine_script_to_chart`, and the server instructions cover charts, drawings, Pine scripts and alerts. |
| Production gaps: audit log, confirmations, real-client matrix, resources, rate limits, lifecycle cases | Open | None of the seven is addressed here; see `docs/mcp-remaining-work.md`, items 1, 2, 3, 8, 10 and 14. The review's ship criteria for audit and confirmation defaults are not met. |

Verified differently from the review: the claim that a burst of requests plans against one snapshot is
true because the snapshot only changes when the runtime drains its queue (at most once a second in the
app, or in the background agent's process), not when a command is queued.

## What is already good

- The listener binds explicitly to IPv4 loopback and the SDK is configured with exact `Host` values, rejection of every present `Origin`, a 256 KiB body limit, a request deadline, and bounded concurrency ([`src/mcp/mod.rs`](../../src/mcp/mod.rs), [`src/mcp/transport.rs`](../../src/mcp/transport.rs)).
- Credentials are 256-bit OS-random tokens; only SHA-256 verifiers are persisted; verifier comparison is constant-time; unauthenticated requests do not get route information ([`src/mcp/auth.rs`](../../src/mcp/auth.rs)).
- Permissions are granular, globally capped, filtered from discovery, checked at tool dispatch, and checked again immediately before app-thread execution ([`src/model/mcp.rs`](../../src/model/mcp.rs), [`src/mcp/tools.rs`](../../src/mcp/tools.rs), [`src/mcp/bridge.rs`](../../src/mcp/bridge.rs)).
- Workspace state stays behind the app-thread bridge. The HTTP/tool threads do not directly lock chart workspaces or write SQLite.
- Queue sizes, Pine worker concurrency, source sizes, candle pages, drawing counts, alert counts, report pages, and returned diagnostics are bounded.
- Saved MCP state is decoded strictly and fails closed rather than replacing unreadable credentials with defaults ([`src/store/mcp.rs`](../../src/store/mcp.rs)).
- The focused suite is substantial: 159 MCP-related tests passed during this review.

These are the right architectural choices. The blocking findings are mostly atomicity and lifecycle holes at the seams between otherwise well-designed components.

## Release-blocking findings

### P1 — Authentication has a check/use race that can return the wrong identity

Evidence: [`src/mcp/state.rs:109`](../../src/mcp/state.rs#L109)

`McpShared::authenticate` verifies the token while holding a read lock, stores the matching **vector index**, releases the lock, then takes a write lock and reads the client at that index to update `last_used_ms` and build `ClientIdentity`.

A client-list replacement can occur between those locks. Revocation uses `retain`, so indices can shift:

1. token B matches client B at index 1;
2. Settings revokes client A at index 0 and publishes the new vector;
3. authentication reacquires the write lock and reads new index 1, which may now be client C;
4. token B is treated as client C, including C's effective permissions.

Revoking B itself can produce the same outcome if a later row shifts into B's old index. This is a narrow race, but it crosses the authentication boundary and can elevate a request.

Required fix:

- Verify all verifiers and update the matched client under one write-lock acquisition. Do not carry an index, reference, or inferred position across lock boundaries.
- Continue scanning every verifier before acting so match position is not exposed through early exit.
- Add a deterministic concurrency test using a barrier/test hook between comparison and identity creation. The assertion must prove that a token can only produce the exact client whose verifier matched, or no identity after revocation.

### P1 — MCP Pine save can overwrite an external edit

Evidence: [`src/app.rs:1785`](../../src/app.rs#L1785), [`src/mcp/pine.rs:426`](../../src/mcp/pine.rs#L426), [`src/pine_studio/store.rs:370`](../../src/pine_studio/store.rs#L370), [`src/pine_studio/mod.rs:1294`](../../src/pine_studio/mod.rs#L1294)

The normal Pine Studio save path uses `ScriptStore::save_if_unchanged`, comparing the file with the editor's last-known disk baseline. The MCP path calls `ScriptStore::save` unconditionally after checking only the hash of the working editor text.

When a script is open, `working_text` deliberately gives the editor precedence over disk. Therefore this sequence loses data:

1. Pine Studio has source A open and dirty as B;
2. another process changes the `.pine` file from A to C;
3. before the editor's external-change watcher reconciles C, an MCP client saves expected revision B;
4. the revision check succeeds against the open editor, then unconditional `save` overwrites C with B.

The current tests explicitly establish that open-editor text outranks disk, but do not test the external-change collision. This bypasses the data-loss protection used by the GUI.

Required fix:

- Have the Pine Studio expose the open editor's source, saved disk baseline, and unresolved external-conflict state as one snapshot.
- Refuse the MCP save while an external conflict is unresolved.
- Commit with `save_if_unchanged` using the editor's saved baseline. Do not use a freshly rescanned file as the expected baseline; that would merely bless the unseen external edit for overwrite.
- Map `ChangedOnDisk` to `CONFLICT` and leave the draft/editor untouched.
- Add tests for external edit with a clean editor, external edit with dirty editor text, and an external edit racing immediately before the write.

### P1 — Alert mutations are not atomic against the state they validate

Evidence: [`src/app.rs:1492`](../../src/app.rs#L1492), [`src/app.rs:1532`](../../src/app.rs#L1532), [`src/model/alert.rs:621`](../../src/model/alert.rs#L621), [`src/model/alert_engine.rs:110`](../../src/model/alert_engine.rs#L110)

MCP validates and plans alert changes against `AlertHost::snapshot()`, then enqueues full-object commands for a different runtime to apply later. Up to eight MCP requests run in one UI frame, while the alert runtime has not necessarily applied any of their commands.

Concrete failures:

- Two identical creates can both see no duplicate, mint different IDs, and enqueue two identical alerts.
- A burst near 500 alerts can make every request pass the cap before the commands apply, exceeding the advertised limit.
- Two partial updates to one alert are each expanded into a full `PriceAlert` based on the same old snapshot; the later save silently loses fields changed by the first.
- Delete followed by an update planned from the old snapshot can delete and then reinsert the alert, because `save_alert` upserts by ID.

`AlertRuntimeCommand` already has `expected_revision`, but constructors set it to `None` and `AlertEngine::apply` never enforces it. The data model has the start of the correct solution but currently provides no compare-and-swap behavior.

Required fix:

- Put alert preconditions into the command applied by the owning runtime: expected snapshot revision plus operation-specific checks (ID still exists, ID absent for create, no identical alert, cap still available).
- Make the runtime return an applied/conflict result correlated to the command rather than inferring success by polling snapshots.
- Prefer patch commands for updates, or enforce the expected alert revision/fingerprint before replacing a full object.
- Test two simultaneous identical creates, updates to disjoint fields, update-vs-delete in both orders, and a burst at the cap.

### P1 — Alert creation reports success when durable enqueue fails

Evidence: [`src/app.rs:350`](../../src/app.rs#L350), [`src/app.rs:1511`](../../src/app.rs#L1511)

`commit_price_alert` discards both `enqueue_alert_command` results and returns `()`. The MCP create path then returns `status: "queued"` regardless of whether the save command reached SQLite. Update, pause, resume, and delete correctly use `queue_alert_command`, which propagates enqueue failure; create is the inconsistent exception.

It also requests notification authorization and starts service setup even if the alert save was not enqueued.

Required fix:

- Return `Result` from alert creation commit and make successful durable enqueue of the `Save` command a prerequisite for success.
- Queue the save and notification-authorization command transactionally if both are required, or make the latter explicitly best-effort only after save succeeds.
- Do not spawn authorization/service work after a failed save enqueue.
- Add a failing-store test that asserts an `APP_UNAVAILABLE` tool error, no success result, and no side effects.

### P1 — Abandoned bridge calls remain armed and may mutate later

Evidence: [`src/mcp/bridge.rs:230`](../../src/mcp/bridge.rs#L230), [`src/mcp/bridge.rs:299`](../../src/mcp/bridge.rs#L299)

The bridge cancels a request only when its own timeout branch runs. If the `BridgeClient::call` future is dropped—for example because a client closes the per-request stream, the MCP request is cancelled, or an outer HTTP future is aborted—the local `Arc<AtomicU8>` is simply dropped. The queued `Request` retains another `Arc`, still in `PENDING`, and the app thread later changes it to `STARTED` and executes it.

That contradicts the module contract that an abandoned request cannot mutate later. It is particularly important for the modern stateless lifecycle, where closing the request stream is the cancellation signal.

Required fix:

- Add an RAII pending-call guard whose `Drop` performs `PENDING -> CANCELLED`. Disarm it only once a reply is received or execution is known to have started.
- Thread the SDK/request cancellation token into bridge waits and Pine worker orchestration where available.
- Add tests that abort a spawned `BridgeClient::call` before inbox drain, cancel while queued behind other requests, and disconnect an HTTP caller before a destructive command begins. The execution closure must not run.

## Important hardening and correctness findings

### P2 — Permission enforcement is duplicated and future tools fail open at dispatch

Evidence: [`src/mcp/tools.rs:76`](../../src/mcp/tools.rs#L76), [`src/mcp/tools.rs:2106`](../../src/mcp/tools.rs#L2106)

Permission knowledge exists in at least three places: `required_permission`, each handler's explicit permission argument, and each bridge call. `call_tool` checks permissions only when `required_permission` returns `Some`; a newly registered tool omitted from that match is hidden from `tools/list` but is still routed when its name is guessed. Its handler then becomes the only remaining protection.

The current tools appear consistently mapped and the tests exercise them, so this is not a present authorization bypass. It is a dangerous maintenance default for a growing security surface.

Recommendation:

- Fail closed for any routed tool without a permission mapping.
- Derive the bridge permission from the command/tool definition rather than accepting an unrelated argument.
- Add one catalog test asserting every router tool has exactly one permission and every mapped name exists in the router.

### P2 — Concurrency limiting happens after authentication; failed authentication is unbounded

Evidence: [`src/mcp/transport.rs:56`](../../src/mcp/transport.rs#L56), [`src/mcp/auth.rs:72`](../../src/mcp/auth.rs#L72)

Axum layers execute outermost-last. The bearer middleware is therefore outside `ConcurrencyLimitLayer`. Arbitrarily many invalid-token requests can concurrently hash input, scan the credential list, allocate responses, and occupy connection/runtime resources before reaching the cap. The server also has no failed-auth or per-client rate limit, which is already listed as remaining work.

Recommendation:

- Put a small transport-level concurrency limit outside authentication as well as a per-client limit inside it.
- Add a bounded failed-auth token bucket keyed conservatively for a loopback service (global plus process/client identity where available).
- Bound header count/size explicitly and add a JSON nesting-depth limit rather than relying only on body bytes.

### P2 — CPU-heavy Pine work can block the UI thread in bursts

Evidence: [`src/app.rs:1444`](../../src/app.rs#L1444), [`src/mcp/pine.rs:426`](../../src/mcp/pine.rs#L426), [`src/host/mcp_host.rs:720`](../../src/host/mcp_host.rs#L720)

The app drains up to eight requests per frame. Saving a script compiles up to 200,000 characters on the app thread; changing instance inputs recompiles that instance's stored source there as well. A permitted client can queue several such calls and create a multi-compile frame stall.

The app-thread compare/commit is necessary, but compilation does not all need to happen inside the commit critical section. Prepare immutable source/hash/schema work on the bounded Pine workers, then perform a short revision recheck and commit on the app thread. If private chart source must be captured on the app thread, snapshot it first, compile off-thread, and compare its hash again before applying.

### P2 — Drawing ID exhaustion can panic or wrap

Evidence: [`src/mcp/chart_data.rs:381`](../../src/mcp/chart_data.rs#L381), [`src/mcp/chart_data.rs:422`](../../src/mcp/chart_data.rs#L422), [`src/host/mcp_host.rs:849`](../../src/host/mcp_host.rs#L849)

`next_free_id` can return `u64::MAX` for a persisted drawing with that ID. `first_id + index` and `first_id + len` then panic in debug builds and wrap in release builds, potentially creating ID zero or duplicates. Remote callers cannot choose new IDs directly, but imported/corrupt/newer-version layout data can reach this path.

Use `checked_add` for the entire reserved range and return `RESOURCE_LIMIT` when IDs are exhausted. Add boundary tests at `u64::MAX - 1` and `u64::MAX`.

### P3 — Fractional ISO-8601 seconds are accepted but discarded

Evidence: [`src/mcp/drawings.rs:187`](../../src/mcp/drawings.rs#L187)

`parse_time` documents optional fractional seconds, validates that the fraction contains digits, then parses only the portion before the decimal point. `...00.999Z` becomes `...00.000Z`. This is mostly hidden when a point snaps to a candle, but it changes out-of-range/future anchors.

Either parse one to nine fractional digits into milliseconds (with a documented rounding rule) or reject fractions and remove the claim that they are supported.

### P3 — Listener status can race startup failure

Evidence: [`src/mcp/mod.rs:32`](../../src/mcp/mod.rs#L32)

`McpServer::start` marks the server `Running` immediately after spawning the thread. Runtime creation and conversion of the std listener happen later in that thread and can fail. A fast thread failure can be overwritten by the parent setting `Running`; a thread panic also leaves stale status because the join result is ignored.

Use a one-shot startup handshake so `start` returns only after the runtime owns the listener, and surface panics/join failures as `Failed`. This also makes the status and endpoint readiness precise.

### P3 — Error and status reporting swallows useful failures

- Persistence of `last_used_ms` ignores SQLite errors after clearing the dirty flag ([`src/host/mcp_host.rs:171`](../../src/host/mcp_host.rs#L171)). The timestamp can be lost indefinitely if the client makes no later request.
- Secondary compile results after Pine update/save are converted with `.ok()`, so an internal compile/store failure silently becomes `compile: null` ([`src/mcp/tools.rs:1146`](../../src/mcp/tools.rs#L1146), [`src/mcp/tools.rs:1200`](../../src/mcp/tools.rs#L1200)). If compilation is optional, return a typed warning/status; otherwise fail the call.
- The `compile_pine_script` description tells clients to use `degenview_add_indicator` for Pine, but the correct tool is `degenview_add_pine_script_to_chart` ([`src/mcp/tools.rs:1210`](../../src/mcp/tools.rs#L1210)). Server instructions also describe only charts despite the much larger surface ([`src/mcp/tools.rs:2100`](../../src/mcp/tools.rs#L2100)).

## Production gaps that should block general release

These are already acknowledged in `docs/mcp-remaining-work.md`; this review agrees they are required before calling the server production-ready:

1. **Audit log and recent activity.** Record client ID/name, tool, target IDs, allow/deny, outcome code, duration, and correlation ID without arguments, source, webhooks, or tokens. Denied and failed calls must be included and storage must be bounded.
2. **Confirmation for high-impact actions, default on.** At minimum: alert deletion, saved Pine overwrite, and applying executable Pine to a chart. Confirmation must bind client, exact operation/targets, revision/hash, and expiry.
3. **Real-client compatibility.** Exercise MCP Inspector and supported desktop/CLI clients, including both the legacy initialize lifecycle and the `2026-07-28` stateless discovery/envelope path that the pinned SDK advertises.
4. **Cancellation and disconnect behavior.** Verify the actual SDK/HTTP cancellation context reaches tools and that no queued mutation survives caller abandonment.
5. **Rate limiting and parser hardening.** Per-client limits, failed-auth limits, JSON depth, and explicit header limits.
6. **Operational lifecycle cases.** Window/tab closure during requests, app shutdown with queued and running work, store contention/failure, port-change rollback, and server-thread failure.
7. **Resources or an explicit non-support statement.** The original scope calls for resource discovery/read where appropriate; either implement permission-equivalent resources or document tools-only support and tested client implications.

The locally generated bearer-token scheme is appropriate for a loopback-only mode, but documentation must continue to state that it is not the full MCP OAuth authorization flow and must never be reused for LAN/remote exposure.

## Test and tooling assessment

Commands run:

```text
cargo test mcp --locked
    PASS — 159 passed, 0 failed

cargo clippy --lib --locked -- -D warnings
    PASS

cargo clippy --lib --all-features --locked -- -D warnings
    NOT COMPLETED — the iCloud Swift build attempted to write
    /Users/nico/.cache/clang/ModuleCache, which the review sandbox forbids
```

The existing tests are broad on validation, permissions, loopback binding, Host/Origin checks, body size, stop/revoke behavior, Pine revision conflicts, drawings, and normal alert lifecycle. The main weakness is that the test harness applies alert commands immediately, unlike the real queued runtime, so it cannot expose the alert TOCTOU failures described above.

High-value missing tests, in order:

1. credential replacement during authentication;
2. dropped/cancelled bridge future before inbox drain;
3. external Pine file change while the script is open and dirty;
4. failing SQLite enqueue during alert create;
5. concurrent alert create/update/delete interleavings against the real command queue;
6. unsupported/mismatched protocol versions and the full 2026 per-request metadata path;
7. server startup-thread failure and port-change rollback;
8. drawing ID exhaustion;
9. authenticated and unauthenticated concurrency/rate-limit stress;
10. a real-client acceptance scenario covering discovery, a read, a mutation, revoke, cancellation, and server disable.

## Recommended remediation sequence

1. Fix the authentication race and add the deterministic race test.
2. Add bridge drop-cancellation and end-to-end disconnect tests.
3. Make alert commands revision-aware and make create enqueue errors visible; replace polling-based success inference with correlated outcomes.
4. Preserve Pine's external-file compare-before-write invariant in the MCP save path.
5. Add audit logging and confirmation policy.
6. Centralize fail-closed permission mapping and move transport resource limits outside auth.
7. Remove app-thread compile bursts and fix numeric/ID/time edge cases.
8. Run the complete required suite outside the sandbox, then test supported real clients before release.

## Ship criteria

I would approve this MCP server for production only when:

- every P1 finding above has a regression test and is fixed;
- audit and confirmation defaults are implemented;
- caller cancellation is proven not to surprise-mutate later;
- real alert-queue concurrency tests pass;
- external Pine edits cannot be overwritten without an explicit user decision;
- the full `cargo test`, all-features clippy, formatting, macOS/Linux CI, and real-client acceptance matrix are green;
- documentation pins the supported protocol versions, authentication model, tools/resources support, and known limitations.

Until then, the feature is suitable for controlled local testing behind its current off-by-default switch, not for a general production-ready claim.
