# Favorites and watchlist production-readiness review

Date: 2026-11-04  
Reviewed revision: `6d85996` (`main`)  
Scope: watchlist and Favorites domain models, sidebar state and interaction, workspace integration,
quote coordination and provider adapters, import/export, SQLite persistence, and iCloud merge/apply
behavior.  
Review type: static review plus focused automated checks; no implementation changes, live-provider
fault injection, multi-device iCloud exercise, long-running resource test, or manual GUI pass.

## Verdict

The Favorites/watchlist implementation has a strong foundation, but it is **not production-ready
yet**. The pure list/layout models are well tested, SQLite writes are transactional, unreadable data
is protected from replacement, quote fetching is kept off the UI thread, and iCloud has explicit
projection and three-way merge support.

The release-blocking concern is the live quote lifecycle. A sidebar that is closed, hidden behind
another tab, or no longer rendered remains subscribed, while an open sidebar requests every market
from every saved list rather than the rows it actually displays. Provider-wide failures can then be
mistaken for bad symbols and recursively expanded into many requests without useful backoff. With
large libraries or several tabs, the feature can keep polling substantial invisible demand and
amplify an authentication, regional, or provider outage.

Correctness also needs hardening. Market identity has several incompatible definitions, partial
batch failures discard good results, invalid stored invariants are accepted, and keyboard actions
can target a selected row hidden by a filter. Persistence APIs routinely suppress domain errors,
which makes several of these cases impossible for the UI to explain.

Severity meanings used below:

- **High:** a realistic workflow can cause uncontrolled background work, request amplification, or
  broad quote unavailability.
- **Medium:** a plausible user-visible correctness, data-integrity, durability, or bounded
  performance defect.
- **Low:** maintainability or hardening work that is unlikely to cause immediate data loss.

## Findings summary

| ID | Severity | Finding |
| --- | --- | --- |
| H1 | High | Quote demand survives sidebar/tab/window invisibility and violates workspace suspension |
| H2 | High | One visible list subscribes every market in every saved list |
| H3 | High | Provider-wide 4xx failures trigger recursive symbol isolation and bypass effective backoff |
| M1 | Medium | A failed page makes successful quote pages get discarded |
| M2 | Medium | Market identity is inconsistent across list membership, Favorites, chart matching, quotes, and sync |
| M3 | Medium | Stored and synced watchlists are decoded but not validated for domain invariants |
| M4 | Medium | Store mutation APIs swallow domain failures and often persist unchanged collections |
| M5 | Medium | A filtered-out selected row can still be opened or deleted from the keyboard |
| M6 | Medium | Several provider adapters erase transport failures, defeating health reporting and backoff |
| M7 | Medium | Local names/import sizes and export filenames are effectively unbounded and unsanitized |
| L1 | Low | Quote and coordinator bookkeeping is never pruned during a long session |
| L2 | Low | Watchlist timestamps claim edit tracking but `updated_at_ms` is never updated |

## High findings

### H1 — Quote demand survives sidebar, tab, and window invisibility

`show_watchlist` publishes demand only while the panel is drawn
([workspace/watchlist.rs](../../src/workspace/watchlist.rs#L13)). The panel itself is skipped as soon
as `watchlist_visible` becomes false
([workspace.rs](../../src/workspace.rs#L614)), so the transition that closes it never calls
`set_demand(..., false)` or `release`. Demand is released only when the entire `ChartWorkspace` is
dropped ([workspace.rs](../../src/workspace.rs#L775)).

The same stale state applies when a tab remains alive but is not the active tab, or when its window
is hidden/occluded and stops drawing. This contradicts the repository's ownership rule that hidden
tabs suspend per-workspace refreshes. The coordinator continues to regard the last submitted
consumer as active and keeps its REST polling and Coinbase socket subscriptions alive.

Impact: opening a watchlist once can leave network traffic, API usage, wakeups, and quote parsing
running until the tab is destroyed. Several inactive tabs accumulate active consumers. This wastes
resources, increases rate-limit pressure, and can make provider health look relevant to work that no
user can see.

Recommendation: drive quote demand from an explicit workspace visibility lifecycle rather than
from the panel renderer. Publish an empty/inactive demand immediately when the sidebar closes, when
the tab/window becomes non-rendered or occluded, and before a workspace is detached from its host.
Keep `Drop::release` as a final safety net. Add integration tests for closing the sidebar, switching
tabs, hiding/covering a window, restoring it, and closing a tab while a fetch is in flight.

### H2 — The selected list requests quotes for the whole library

Every rendered sidebar builds its demand by flattening `instruments()` from **all** watchlists
([workspace/watchlist.rs](../../src/workspace/watchlist.rs#L27)). It ignores the sidebar's selected
list, text/flag filter, and collapsed sections. The domain already has `visible_instruments`, whose
comment says it exists for quote subscription decisions, but it is unused
([model/watchlist.rs](../../src/model/watchlist.rs#L482)).

Impact: a user viewing a ten-symbol list while holding twenty imported lists of 5,000 entries asks
the coordinator to retain and poll the entire library. Prediction markets are fetched one series at
a time, Coinbase REST fallback performs one request per product, and DEX pairs may need preliminary
network resolution, so this is not merely an oversized in-memory set. It can turn normal sidebar
use into sustained provider load and make the visible rows stale because invisible rows consume the
same rate limits.

Recommendation: have the sidebar expose the selected list and the exact quote-relevant rows after
collapse/filter presentation has been resolved. Subscribe only visible market rows; exclude
portfolio cards and unsupported CMC instruments before they reach provider planning. Debounce
demand changes and avoid rebuilding/reconciling an identical set every frame. Add a test with two
large lists, a collapsed section, a filter, and a portfolio card that asserts only visible market
identities are demanded.

### H3 — Provider-wide 4xx responses are amplified as bad-symbol searches

The shared HTTP adapter maps every 400–499 response except 429 to `QuoteFailure::Rejected`
([provider.rs](../../src/service/watchlist_quotes/provider.rs#L100)). The engine interprets that as
evidence that one or more symbols are invalid and recursively bisects the request until individual
symbols are isolated ([engine.rs](../../src/service/watchlist_quotes/engine.rs#L546)).

That classification is unsafe for batch-global failures such as Alpaca 401/403 credentials,
Binance regional denial, an unavailable endpoint, or a provider rejecting the request shape. A
batch of 100 symbols can become many repeated requests carrying the same invalid authorization or
regional condition. If every leaf is rejected, `Outcome.failure` is empty and `apply` sees an empty
successful response, clears backoff, and retries on the normal source interval
([engine.rs](../../src/service/watchlist_quotes/engine.rs#L401)). The existing
`CredentialsMissing` handling covers absent Alpaca keys, not invalid/revoked keys.

Impact: a persistent account or regional failure can produce a request storm and repeated provider
load while every row remains unavailable. Combined with H1 and H2, the user need not have the
affected rows visible for this to continue.

Recommendation: classify HTTP failures by source and status. Authentication, authorization,
regional, endpoint, and malformed-batch errors must remain source-level failures with bounded
backoff; only a documented symbol-level rejection should enter binary isolation. Put a strict
per-round request budget around isolation and preserve the original failure if every branch fails
the same way. Add tests for 401, 403, 404 endpoint, 451, 429, one bad symbol among good symbols, and
an entire rejected batch.

## Medium findings

### M1 — A failed page discards good quote pages

Large source batches are split into provider-sized pages. `fetch_chunked` merges successful quotes
but also retains the first page failure in the aggregate outcome
([engine.rs](../../src/service/watchlist_quotes/engine.rs#L556)). `Engine::apply` branches on that
single failure before inspecting `outcome.quotes`, marks every requested key stale/unavailable, and
does not publish the successful pages ([engine.rs](../../src/service/watchlist_quotes/engine.rs#L370)).

Impact: one transient failure in a 251-coin CoinGecko list, or one failed Binance page in a large
library, blanks or stales all pages rather than preserving the majority that succeeded. Provider
health likewise reports a total failure.

Recommendation: model page results explicitly. Apply successful quotes, mark only keys from failed
pages with their fallback state, and schedule retry/backoff for those pages without discarding good
data. Add a regression test where the middle of three pages fails and the first/last page quotes
remain live.

### M2 — Market identity has incompatible definitions

The domain documentation says identity is provider plus provider ID, but the implementation uses
several different comparisons:

- list duplicate checks and quote/flag keys use `InstrumentId::key`, which normalizes case but does
  not canonicalize bare Binance/Coinbase symbols
  ([instrument.rs](../../src/model/instrument.rs#L20));
- chart matching uses `is_same_market`, which compares canonical `api_symbol`
  ([instrument.rs](../../src/model/instrument.rs#L59));
- grid Favorites and membership snapshots use exact derived `Eq`/`Hash` on the stored struct
  ([workspace/grid.rs](../../src/workspace/grid.rs#L300));
- iCloud merge constructs its own raw JSON source/symbol key without either normalization
  ([sync/merge.rs](../../src/sync/merge.rs#L203)).

Examples such as Binance `BTC` versus `BTCUSDT`, Coinbase `ETH/USD` versus `ETH-USD`, or casing that
differs between Macs can therefore be the same chart market but two watchlist/Favorites rows. The
star can appear off while an equivalent favorite exists; removing one representation leaves the
other; sync can merge or duplicate differently from the local store.

Recommendation: introduce one canonical, serializable market identity derived from the exact
provider API identifier and use it for equality, hashing, flags, quote keys, list membership,
chart matching, and sync merge keys. Normalize at every ingress boundary and repair existing lists
deterministically, retaining the richest metadata when duplicates collapse. Test every supported
provider's aliases, casing rules, import/search/chart/Favorites round trips, and two-Mac merges.

### M3 — Persisted lists are decoded but not checked for invariants

`Watchlist::validate` checks duplicate entry IDs and market identities, but is compiled only in
tests ([model/watchlist.rs](../../src/model/watchlist.rs#L740)). `WatchlistStore` accepts any payload
that Serde can decode ([store/watchlist.rs](../../src/store/watchlist.rs#L27)). It also only ensures
that at least one list is marked Favorites; it does not reject or normalize multiple Favorites
lists, duplicate list IDs, duplicate entry IDs, or duplicate markets.

The remote validator bounds the entry count but does not validate entry shapes or these invariants
([sync/validate.rs](../../src/sync/validate.rs#L119)). Sync merge silently filters malformed and
duplicate-key entries while constructing its merge view
([sync/merge.rs](../../src/sync/merge.rs#L222)), which can turn corruption or a newer entry shape
into data loss rather than a preserved invalid record.

Impact: a damaged database, older development build, or malformed cloud record can produce
ambiguous row operations and Favorites selection. Some paths remove only the first matching
market, while list persistence can later write the broken state back. Unknown remote entries may
be stripped and uploaded as a valid-looking merged list.

Recommendation: make validation a production domain boundary. Strict load should reject the entire
collection without publishing defaults when any list violates invariants. Remote validation should
either accept a completely understood list or retain it in the sync inbox unchanged; it must not
silently sanitize unknown entries during merge. Define a recovery path for multiple Favorites and
duplicate identities, with tests for local corruption and remote malformed/newer rows.

### M4 — Store mutations swallow domain failures

Most `WatchlistStore` mutations return `()` and deliberately discard model results
([store/watchlist.rs](../../src/store/watchlist.rs#L220)). Duplicate additions, missing rows,
invalid section names, invalid drop targets, and failed moves are therefore indistinguishable from
success. Several methods call `persist()` even when the model rejected the operation, rewriting the
entire watchlist document collection. `create_list` and `rename_list` bypass
`Watchlist::validated` altogether ([store/watchlist.rs](../../src/store/watchlist.rs#L174)).

Impact: stale UI actions or identity collisions fail silently; callers cannot display a meaningful
error. Unchanged full-collection writes add UI-thread SQLite work and can still interact with sync
bookkeeping. The API is easy for future non-UI callers to misuse because its invariants depend on
the current sheet trimming text first.

Recommendation: return a typed `Result<MutationOutcome, WatchlistError>` from every mutation,
validate names and target IDs in the store/domain layer, and persist only when the candidate state
actually changed. Build the candidate copy first, save it, then publish it, matching other hosts'
persist-before-publish contract. Make workspace handlers consume and show each failure. Add tests
for duplicate add, invalid/stale IDs, unchanged toggles, invalid names, and injected SQLite failure.

### M5 — Keyboard actions can target a row hidden by the filter

Changing the text or flag filter does not reconcile `selected_entry`. The renderer builds filtered
rows, but its Enter/Delete handler resolves the selection against every instrument in the stored
list rather than the visible rows ([watchlist_sidebar.rs](../../src/ui/watchlist_sidebar.rs#L916),
[watchlist_sidebar.rs](../../src/ui/watchlist_sidebar.rs#L1077)). The detail panel follows the same
stored selection.

Impact: select a row, apply a filter that hides it, then press Delete/Backspace: an invisible market
is removed. Enter can likewise open a market that is not in the result set. If the filter yields no
rows, `content` returns before the keyboard handler, so behavior also changes depending on whether
some unrelated row still matches.

Recommendation: clear or move selection whenever filtering/collapse/list changes hide the selected
row. Keyboard commands must resolve against the current visible row model, not the backing list.
Add UI-state tests for text filter, flag filter, collapsed sections, no-results state, and deleting a
visible versus hidden selection.

### M6 — Provider adapters erase transport failures

Several adapters convert request failures into an empty successful map:

- Coinbase ignores every per-product failure except 429
  ([provider.rs](../../src/service/watchlist_quotes/provider.rs#L129));
- DEXScreener ignores each failed chain/page request
  ([provider.rs](../../src/service/watchlist_quotes/provider.rs#L167));
- prediction-market fetching ignores every failed candle fetch
  ([provider.rs](../../src/service/watchlist_quotes/provider.rs#L208)).

The engine then records the generic “provider returned no quote” state, clears failure backoff, and
retries at the normal refresh interval. Partial results cannot say which requests failed, and
provider-health diagnostics lose the actual cause.

Recommendation: return per-request/page success and failure information from `QuoteProvider`.
Preserve rate limits, network errors, authorization errors, and parse errors through the engine;
back off failed work without suppressing successful work. Add deterministic mixed-success and full
outage tests for each provider adapter.

### M7 — Names, imports, and export filenames lack consistent bounds

Watchlist and section validation rejects only whitespace-only text
([model/watchlist.rs](../../src/model/watchlist.rs#L759)). The prompt has no character/control limit,
plain-text parsing collects the complete input before processing, and local lists have no equivalent
of sync's 5,000-entry/200-character remote bounds. A locally created oversized list may work until
sync refuses the record. Export interpolates the unrestricted list name directly into the suggested
filename ([workspace/watchlist.rs](../../src/workspace/watchlist.rs#L156)).

Impact: pasted or imported content can create large UI-thread allocations and documents, very long
menu/row content, invalid suggested filenames, and a list that cannot sync to another Mac. Control
characters and path separators can produce confusing presentation or save-dialog defaults.

Recommendation: define shared local/remote limits for list count, entries per list, name/section
characters, import bytes/lines, and field lengths. Reject control characters, normalize whitespace,
and sanitize only the suggested filename while retaining the display name. Parse imports with a
bounded iterator and report truncation/refusal explicitly. Test boundary and over-bound inputs.

## Low findings

### L1 — Quote state grows for the lifetime of the coordinator

`Engine::known`, `received`, `rejected`, `chains`, `unresolved_chains`, and
`coinbase_subscribed_at`, plus the shared `WatchlistQuoteBook`, retain keys after every consumer
stops requesting them ([engine.rs](../../src/service/watchlist_quotes/engine.rs#L94)). Reconciliation
clears the active Coinbase product set but does not evict per-market state.

Impact: repeated imports, deletions, and browsing of many markets make session memory grow. Old
quotes also reappear immediately if the same identity returns, even when they are far outside the
useful freshness window.

Recommendation: reference-count active identities and evict inactive transient state after a
bounded grace period. Keep only reusable metadata whose cache policy and maximum size are explicit.
Add a churn test that subscribes/releases many unique symbols and asserts bounded retained state.

### L2 — `updated_at_ms` is never maintained

`created_at_ms` and `updated_at_ms` are initialized together and copied into sync payloads
([model/watchlist.rs](../../src/model/watchlist.rs#L436)), but no mutation updates
`updated_at_ms`. Sync takes the maximum timestamp during merge, so the field appears meaningful
without representing the last edit ([sync/merge.rs](../../src/sync/merge.rs#L260)).

Impact: diagnostics and future sorting/conflict UI cannot trust the timestamp. It also creates
unnecessary stored/synced surface with no valid contract.

Recommendation: either update it once per successful logical mutation using an injected/testable
clock, or remove it from the model and sync payload if record revisions are the intended edit
ordering. Add deterministic timestamp tests for create, mutate, failed save, duplicate, and merge.

## Strengths worth preserving

- The list and layout engines are pure and have broad tests for sections, sorting, filtering,
  imports, moves, and formatting.
- Strict document reads prevent an undecodable SQLite row from being silently replaced with an
  incomplete collection ([store/mod.rs](../../src/store/mod.rs#L299)).
- Persistence failures restore the stored watchlist state instead of publishing an unsaved edit
  ([store/watchlist.rs](../../src/store/watchlist.rs#L120)).
- SQLite replacement and sync outbox tracking share one savepoint, preserving atomicity.
- Quotes are transient and fetched off the render thread; the coordinator merges duplicate demand
  and keeps provider backoff isolated by source.
- Favorites has an explicit singleton sync identity while retaining each Mac's local row ID.
- The sidebar uses the shared fitted menu system and has width tests, matching repository UI
  guidance.

These are good production-oriented choices. The recommended changes should keep the pure-domain and
persist-before-publish shape while making lifecycle, identity, error, and validation contracts
explicit.

## Verification performed

- `cargo test watchlist --lib` — **112 passed**, 0 failed.
- `cargo clippy --lib -- -D warnings` — **passed**.

The passing suite confirms substantial unit coverage but does not exercise the workspace lifecycle
that causes H1/H2, real HTTP status behavior, partial page faults, long-running state churn, or a
two-Mac malformed/alias identity merge. Before release, add those automated scenarios and perform a
manual GUI pass with opened menus, filtering/keyboard deletion, several tabs/windows, a large
imported library, invalid Alpaca credentials, provider outage, and iCloud changes arriving while a
sidebar is open and hidden.

## Validation and fix status

Validated against `main` at `d782f68` (2026-11-04): every finding holds. Corrections and extra defects
found while validating:

- **H1:** the engine already supports an inactive consumer; the workspace never uses it. The alert
  editor owner, the portfolio feed and the Binance live feed share the pattern.
- **H2:** the sidebar's own row model (`watchlist_layout::rows`) is the right demand source; the
  unused `visible_instruments` only knew about collapse.
- **H3:** worst case 2N-1 serial requests per round (199 for Binance, 399 for Alpaca), with no stop
  after a 429. CoinGecko never answers a symbol-level 4xx.
- **M1:** also loses the good half of a bisected batch and Coinbase's partial results.
- **M2:** Binance `BTC` vs `BTCUSDT` is reachable through text import and MCP `add_chart`. Coinbase
  `ETH/USD` is not reachable through import or search. A stored `btcusdt` against a `BTCUSDT` chart
  leaves the star stuck.
- **M3:** duplicate list ids cannot reach disk; a second Favorites needs a corrupt database. Sync loss
  needs a three-way conflict on a list holding an unparseable entry.
- **M4:** the sync-bookkeeping impact is overstated (unchanged rows are not queued); the UI-thread
  write remains.
- **M5:** also reachable through a collapsed section, and probably through Enter in the filter box.
- **M6:** DEXScreener swallows 429 too; a transient chain lookup failure is permanent for the session.

| ID | Fix |
| --- | --- |
| H2, M5 | Sidebar draws rows once, reconciles the highlight, publishes demand from the drawn rows |
| H1 | Sidebar demand is a 3 s lease renewed per drawn frame; closing the sidebar suspends at once |
| H3, M1, M6 | Status classification, isolation budget, rate-limit short-circuit, per-request failures in `Outcome` (a transient DexScreener chain lookup failure staying permanent is not yet fixed) |
| L1 | Idle markets are forgotten after a 10 min grace period, book included |
| M2 | Pending |
| M3, M4, M7, L2 | Pending |
