# Portfolio functionality production-readiness review

Date: 2026-11-04

## Verdict

The portfolio implementation is **not production-ready yet**. The UI/state split, copy-on-write
persistence, strict SQLite decoding, background quote/history work, and sync conflict handling are
all solid foundations. However, the accounting path currently produces materially wrong financial
figures in common cases, and the history builder can permanently publish incomplete or
time-inconsistent data.

Release should be blocked on the High findings below. In particular, fully closed trades lose their
realized P&L, ordinary decimal quantities can be rejected because accounting uses exact `f64`
comparisons, daily history uses a price and ledger cutoff from different moments, and range P&L can
report a same-day deposit as profit.

## Scope

Reviewed the portfolio domain and editor/CSV models, ledger and accounting, valuation and history
services, host coordination, SQLite persistence, iCloud application logic, portfolio cards, and the
main Overview/Holdings/Transactions/Statistics presentation paths. The review was static plus the
repository's automated checks; it did not exercise live providers or perform the ignored visual
galleries in a real app window.

## Status

Updated after validating every finding against the code and fixing them in six stacked pull
requests (each branch passes `cargo test`, `cargo fmt --check` and `cargo clippy -D warnings`).
None of this has been looked at in a real window or run against live providers.

| ID | Status | Pull request | Note |
| --- | --- | --- | --- |
| H1 | Fixed | [#304](https://github.com/degenview/degenview/pull/304) | `holdings_and_closed` returns closed positions; statistics and snapshots count their realized P&L. |
| H2 | Mitigated | [#304](https://github.com/degenview/degenview/pull/304) | Tolerance of 1e-12 of the position's peak quantity, not a decimal type. A decimal migration changes the stored and synced form and is still open. |
| H3 | Fixed | [#307](https://github.com/degenview/degenview/pull/307) | A point uses the last daily candle that had finished by its instant. Today's point is priced at yesterday's close. |
| H4 | Fixed | [#307](https://github.com/degenview/degenview/pull/307) | Results carry a fingerprint of their transactions; a rebuild missing an asset is stored and retried every 10 minutes. Broader than described: any edit dated at or after `from` was accepted, not only a same-day one. |
| H5 | Fixed | [#308](https://github.com/degenview/degenview/pull/308) | Other-currency candles converted per day; time-range fetch past 1,000 days. Range fetch not run against live providers; the 1D chart is still dollar-quoted markets only. |
| H6 | Fixed | [#305](https://github.com/degenview/degenview/pull/305) | `PortfolioPeriodChange::new` takes today's net contributions. |
| H7 | Fixed, narrower | [#306](https://github.com/degenview/degenview/pull/306) | Finite values were already enforced at the entry points and a zero price is deliberate (airdrops, rewards), so only a negative price, an empty asset, a non-finite fee, the missing portfolio check on update/import, the move-away check and the CSV cases were real. |
| H8 | Fixed | [#306](https://github.com/degenview/degenview/pull/306) | The page and cards show the accounting error; history is not rebuilt into zeros. Strict decoding and sync gating already covered part of this. |
| M1 | Partly fixed, latent | [#306](https://github.com/degenview/degenview/pull/306) | No UI path moves a transaction between portfolios. A move now drops the source portfolio's stored history on sync, and the local ledger replays the source. The sync apply still does not validate the source ledger. |
| M2 | Fixed, narrower | [#305](https://github.com/degenview/degenview/pull/305) | Quote demand and the unpriced count use open positions. The 8 s delay only applied to a symbol that never answers. |
| M3 | Fixed | [#305](https://github.com/degenview/degenview/pull/305) | The previous-day price uses the FX rate from a day ago, falling back to today's. |
| M4 | Partly fixed | [#307](https://github.com/degenview/degenview/pull/307) | History follows the zone per instant. The transaction editor still shows a past time with today's offset. Non-macOS builds and tests read UTC, so DST is tested with an injected zone. |
| M5 | Fixed | [#306](https://github.com/degenview/degenview/pull/306) | `unique_copy_name`; CSV import reports an ambiguous name. Uniqueness for create and rename is still enforced only in the sheets. |
| M6 | Fixed for selection and history | [#309](https://github.com/degenview/degenview/pull/309) | Transaction edits still save the whole ledger because sync's diff depends on it. No 10k/100k benchmark was run, so the saving is unmeasured. |
| M7 | Not a bug | none | `docs/icloud-sync.md` and `src/sync/ids.rs`: ids at or above 2^24 sync unchanged by design, so nanosecond transaction ids are supported. The statement that the guidance requires counter ids for every transaction was wrong. Left unchanged. |
| L1 | Fixed | [#304](https://github.com/degenview/degenview/pull/304) | The amount is `None` when the latest price is 0. |
| L2 | Fixed | [#305](https://github.com/degenview/degenview/pull/305) | Preference errors reach the error sheet. The history save error was already surfaced through `mutate`; the retry loop is now paused. |

The pull requests are stacked: #304, then #305, #306, #307, #308 and #309, each on the one before.
Until they merge, the sections below describe the code as reviewed.

Still open: a decimal quantity type (H2), validating the source ledger inside the sync apply (M1),
the editor's offset for past times (M4), the intraday chart for non-dollar markets, a benchmark
for M6, and the manual real-window pass in the production gate below.

## Findings summary

| ID | Severity | Finding |
| --- | --- | --- |
| H1 | High | Fully closed positions lose all realized P&L |
| H2 | High | Exact binary floating-point accounting rejects valid decimal transactions and leaves dust |
| H3 | High | Daily history combines end-of-day prices with start-of-day holdings, creating look-ahead and inconsistent snapshots |
| H4 | High | Partial history fetches are committed as final and are not retried |
| H5 | High | Cross-currency and older-than-1,000-day portfolios cannot produce correct history |
| H6 | High | Range P&L treats contributions after the last history point as market profit/loss |
| H7 | High | Transaction validation does not enforce the persisted domain invariants |
| H8 | High | Accounting failures are silently rendered and persisted as an empty/zero portfolio |
| M1 | Medium | Remote moves between portfolios validate and invalidate only the destination ledger |
| M2 | Medium | Closed assets remain in live quote demand and make healthy portfolios look partially unpriced |
| M3 | Medium | Reporting-currency 24-hour change ignores FX movement |
| M4 | Medium | A fixed current UTC offset is used for historical dates, so DST history and displayed transaction times are wrong |
| M5 | Medium | Duplicating a portfolio can create duplicate/oversized names and ambiguous CSV round trips |
| M6 | Medium | Every mutation, including selection, rewrites the complete transaction and history store on the UI thread |
| M7 | Medium | Normal transaction creation bypasses the documented per-database counter ID scheme |
| L1 | Low | A zero latest price makes Top Performers calculate an infinite amount |
| L2 | Low | Several persistence failures are deliberately ignored, leaving UI state different from durable state |

## High findings

### H1 — Fully closed positions lose all realized P&L

`holdings()` removes every position whose quantity is exactly zero before constructing
`PortfolioHolding` ([portfolio_accounting.rs](../../src/model/portfolio_accounting.rs#L43)). The
realized P&L is stored only on that internal position and is consequently discarded. Statistics
then sum realized P&L only from the surviving open holdings
([portfolio_statistics.rs](../../src/model/portfolio_statistics.rs#L202)), and historical snapshots
do the same ([portfolio_history.rs](../../src/model/portfolio_history.rs#L119)).

Impact: buy 1 unit at 100 and sell it at 150, and the portfolio reports zero realized profit after
the position closes. The realized result reappears if the asset is later reopened, which makes the
reported total non-monotonic and especially confusing. Win/loss counts and asset results also omit
closed trades.

Recommendation: make accounting return a richer projection, for example open holdings plus
per-asset and total realized results. Do not represent realized performance only as a field on an
open holding. Add full-close, partial-close, close-and-reopen, fee, and multi-asset regression tests
covering statistics and snapshots.

### H2 — Exact binary floating-point accounting rejects valid decimal transactions and leaves dust

All quantities and monetary values use `f64`. Disposal checks `position.quantity < quantity` and
zeroes the basis only for exact equality ([portfolio_accounting.rs](../../src/model/portfolio_accounting.rs#L218)).
This is not safe for user-entered decimal assets. A representative valid sequence—buy `0.3`, sell
`0.1`, sell `0.2`—can leave `0.19999999999999998` after the first sale and reject the second sale as
insufficient holdings. Other sequences retain tiny positions, cost basis, allocation, or P&L dust.

Impact: valid sells, deletes, edits, imports, and sync merges can be rejected depending on decimal
decomposition. Exact `== 0.0` filters elsewhere amplify the problem.

Recommendation: use an exact decimal/fixed-point representation for persisted quantities, prices,
fees, basis, and realized P&L, with explicit currency/asset scales and checked arithmetic. A small
epsilon patch is not a sufficient financial-accounting contract, though a carefully bounded
normalization layer may be a short-term compatibility bridge. Migration and sync compatibility need
explicit tests before changing the stored form.

### H3 — Daily history combines prices and holdings from different moments

Daily snapshots are timestamped at local midnight. The builder selects the last daily bar whose
**open** is at or before that midnight, then uses the bar's close
([portfolio_history.rs](../../src/model/portfolio_history.rs#L61),
[portfolio_history.rs](../../src/model/portfolio_history.rs#L88)). For the bar opening at the
snapshot timestamp, that close belongs to the end of the day (or is the current forming close).
Accounting, however, includes transactions only through the midnight timestamp
([portfolio_history.rs](../../src/model/portfolio_history.rs#L119)).

Impact: a historical point can use a future end-of-day price while excluding transactions made
during that day. Today's point uses a forming daily close but excludes every transaction after
midnight. This distorts value, P&L, extrema, chart colour, and exported history.

Recommendation: define one unambiguous snapshot instant and apply it to both events and prices.
For completed daily points, use only candles with `close_time <= snapshot_time` and transactions
through that same instant. Prefer omitting today's daily point and appending the separately computed
live point. Tests should cover a purchase during the day, a current forming candle, and provider
bars whose UTC boundary differs from local midnight.

### H4 — Partial history fetches are committed as final and are not retried

The rebuild treats the job as failed only when **every** fetched series is empty. If one of several
assets succeeds, snapshots are built with missing assets and returned as success
([portfolio_history.rs](../../src/service/portfolio_history.rs#L195)). Polling removes retry state for
any `Some(snapshots)` result ([portfolio_history.rs](../../src/service/portfolio_history.rs#L146)),
and storing the result clears the portfolio's invalidation marker
([portfolio_ledger.rs](../../src/model/portfolio_ledger.rs#L316)).

Impact: one transient provider/rate-limit/network failure in a multi-asset portfolio can permanently
remove that asset from all rebuilt history. No later frame retries it unless an unrelated ledger
edit invalidates history again. A rebuild result is also accepted by comparing only its `from`
timestamp ([portfolio_host.rs](../../src/host/portfolio_host.rs#L150)); an edit made while the job is
running can still accept stale output when it invalidates from the same day.

Recommendation: carry a generation or hash of the exact portfolio/transaction basis through the
job and compare it on completion. Return per-asset fetch status; publish only a complete generation,
or persist explicit partial state while retaining retry work for the missing assets. Add deterministic
tests for one-of-two fetch failures and a same-day edit during an in-flight rebuild.

### H5 — Cross-currency and older portfolios cannot produce correct history

The history service drops every asset whose quote currency differs from the portfolio base currency
([portfolio_history.rs](../../src/service/portfolio_history.rs#L203)). If all assets differ, it marks
the result hopeless and suppresses retries forever. This is common—for example, a EUR portfolio
holding a Binance USD/USDT market—even though live valuation already has an FX service.

Separately, history requests are capped at 1,000 daily bars
([portfolio_history.rs](../../src/service/portfolio_history.rs#L23),
[portfolio_history.rs](../../src/service/portfolio_history.rs#L48)) while the builder still emits
points from the first transaction. A portfolio older than roughly 2.7 years therefore gets a leading
run of unpriced/zero points rather than genuine `ALL` history.

Impact: the advertised 1Y/ALL value chart and exported history are empty or materially incomplete
for normal non-USD portfolios and long-held portfolios.

Recommendation: fetch historical FX alongside candles and convert each asset at each snapshot
instant. Page daily candles until the requested start is covered (with bounded cancellation and
progress), or explicitly limit the UI/export range rather than manufacturing zeros. Do not classify
a supported cross-currency portfolio as hopeless.

### H6 — Range P&L uses stale contribution totals

`PortfolioPeriodChange::new` pairs today's live value with `last.net_contributions` from the final
history point ([portfolio_statistics.rs](../../src/model/portfolio_statistics.rs#L134)). The Overview
and value-chart card both call it without current contributions
([portfolio_overview.rs](../../src/ui/portfolio_overview.rs#L356),
[portfolio_card_host.rs](../../src/host/portfolio_card_host.rs#L180)).

Impact: a deposit/buy after the most recent daily or 30-minute point is reported as profit; a
withdrawal/sale is reported as loss. The defect is most visible immediately after a transaction,
exactly when users are likely to inspect the result.

Recommendation: calculate current net contributions from the projected transactions and pass that
explicitly with current value. Treat the first and current endpoints symmetrically. Add tests for a
deposit, withdrawal, buy, and sell after the last stored point.

### H7 — Transaction validation does not enforce persisted domain invariants

The central validator checks positive quantity, price presence for buy/sell, and non-negative fee,
but it does not require finite values, a positive price, valid asset identity, or a real target
portfolio ([portfolio_accounting.rs](../../src/model/portfolio_accounting.rs#L95)). The editor
explicitly permits zero price/total ([portfolio_editor.rs](../../src/model/portfolio_editor.rs#L448)).
CSV parsing accepts any finite signed price, silently turns an invalid fee into zero, and accepts an
empty or source-inconsistent `asset_id` ([portfolio_csv.rs](../../src/model/portfolio_csv.rs#L370)).

The mutation paths are also inconsistent: add verifies the portfolio, while update and import do
not ([portfolio_ledger.rs](../../src/model/portfolio_ledger.rs#L182),
[portfolio_ledger.rs](../../src/model/portfolio_ledger.rs#L203),
[portfolio_ledger.rs](../../src/model/portfolio_ledger.rs#L292)). Moving a purchase to another
portfolio validates only the destination and can leave dependent sells invalid in the source.

Impact: negative/zero cost basis, merged empty-key assets, orphan transactions, or an impossible
source ledger can be persisted. Some of these paths are reachable through edited CSV files today;
others are reachable through host/sync APIs and make future UI changes dangerous.

Recommendation: create one domain-level `validate_transaction` plus `validate_snapshot` boundary
used by editor, import, host mutation, load, and sync. Validate finite positive prices where a type
requires price, finite non-negative fees, source-qualified non-empty asset identity, portfolio
existence, unique IDs/external IDs, and all affected source/destination ledgers before persistence.

### H8 — Accounting failures are silently presented and stored as zero

The main page converts any accounting error to an empty holdings vector
([portfolio_host.rs](../../src/host/portfolio_host.rs#L882)); cards do the same
([portfolio_card_host.rs](../../src/host/portfolio_card_host.rs#L140)), and historical
snapshot construction calls `.ok()` then sums the absence into zero
([portfolio_history.rs](../../src/model/portfolio_history.rs#L119)). Initial store loading validates
JSON decoding but not ledger semantics.

Impact: after corruption, an invalid sync edit, or floating-point rejection, the application can
show a zero-value/empty portfolio instead of warning that the ledger cannot be evaluated. A history
rebuild can persist zero snapshots for the same condition. This hides data-integrity failures and
makes them look like real financial state.

Recommendation: propagate a typed accounting failure into the page/card status and never replace
it with an empty value. Validate the full ledger on load and before every save; keep the last known
good projection when a transient projection fails. History jobs should fail rather than serialize
zero when accounting fails.

## Medium findings

### M1 — Sync validates only the destination of a cross-portfolio move

Incoming transaction upserts are staged under the new `portfolio_id`, and `finish_ledgers` groups
and validates only that portfolio ([portfolio.rs](../../src/sync/apply/portfolio.rs#L105),
[portfolio.rs](../../src/sync/apply/portfolio.rs#L306)). If the previous row belonged to a different
portfolio, removing it can invalidate the source ledger and its history is not correctly handled.

Track both old and new portfolio IDs for an upsert, validate both prospective ledgers atomically,
and invalidate history from the appropriate old/new timestamps on both sides.

### M2 — Closed assets remain live quote dependencies

Quote demand and `unpriced_asset_count` are built from every unique asset in every historical
transaction, not from current non-zero positions
([portfolio_quotes.rs](../../src/service/portfolio_quotes.rs#L342),
[portfolio_quotes.rs](../../src/service/portfolio_quotes.rs#L440)).

A fully sold, delisted, or unsupported asset can therefore delay initial valuation for eight
seconds and permanently show the orange “partly unpriced” status despite having no current exposure.
Build current quote demand from the accounting position set; keep historical assets only in the
history-fetch path.

### M3 — Reporting-currency day change ignores FX movement

The previous-day asset price is converted with the **current** FX rate, the same rate used for the
current price ([portfolio_quotes.rs](../../src/service/portfolio_quotes.rs#L461)). A USD asset that
is flat while USD/EUR moves is reported as flat in an EUR view even though its EUR value changed.

Use an FX observation corresponding to the previous-price reference instant, and define/test the
exact 24-hour/previous-close semantics across market and FX calendars.

### M4 — Historical local dates use today's fixed UTC offset

The host asks for the UTC offset at `now` and passes that single value through an entire historical
rebuild ([portfolio_host.rs](../../src/host/portfolio_host.rs#L1559)). `start_of_day` then treats the
offset as constant ([portfolio_history.rs](../../src/model/portfolio_history.rs#L24)), despite a
comment claiming 23/25-hour DST days. Past transaction editor times use the same current offset.

Impact: points around DST boundaries are bucketed/labeled an hour incorrectly, and old transaction
times display with the wrong seasonal offset. Pass a time-zone identifier or an offset lookup by
instant, not a single current offset.

### M5 — Portfolio duplication breaks the unique-name invariant

Duplication always appends `" Copy"` without normalization, length enforcement, or disambiguation
([portfolio_ledger.rs](../../src/model/portfolio_ledger.rs#L91)). Repeating it creates multiple
`Main Copy` portfolios; a 40-character source exceeds `MAX_NAME_LENGTH`. CSV import resolves a
portfolio name by the first case-insensitive match
([portfolio_csv.rs](../../src/model/portfolio_csv.rs#L410)), so export/re-import can route rows to the
wrong duplicate.

Generate a normalized unique name (`Copy`, `Copy 2`, …) within the maximum length, and enforce the
same invariant in the ledger rather than only in sheets.

### M6 — Simple mutations rewrite the complete ledger on the UI thread

Every host mutation clones the full snapshot and calls `save_portfolio_ledger`
([portfolio_host.rs](../../src/host/portfolio_host.rs#L346)). The store deletes/reinserts every
transaction and every history point ([portfolio.rs](../../src/store/portfolio.rs#L36)), even for a
local selection change. This work holds the store lock and runs on the app thread.

Impact: large imported portfolios and multi-year history will produce frame stalls and excessive
SQLite/JSON churn. Add targeted store mutations or dirty-set persistence (selection-only setting,
single transaction/document changes, append/replace history suffix). Benchmark realistic 10k/100k
transaction ledgers and multi-portfolio history.

### M7 — Transaction creation bypasses the documented counter ID scheme

Repository guidance says portfolio/transaction engines must mint per-database counter IDs below
`2^24` so sync can tag them. The ledger has such a counter, but normal new/duplicate transaction UI
paths use the watchlist nanosecond `new_id()` instead
([portfolio_host.rs](../../src/host/portfolio_host.rs#L1138)). Imports do the same through an injected
`new_id` callback.

This is internally inconsistent with `PortfolioLedger::mint` and the sync ID design. Move all ID
allocation behind the ledger/store allocator, enforce exhaustion, and add tests that every local
portfolio and transaction ID is below `LEGACY_LIMIT` before sync projection.

## Low findings

### L1 — Top Performers can calculate infinity

Performance accepts a finite latest price of zero, computes `change == -1`, then divides the amount
by `1 + change` ([portfolio_card.rs](../../src/model/portfolio_card.rs#L185)). Reject non-positive
latest prices or compute the start/end position value without that algebraic division; assert all
published card values are finite.

### L2 — Preference and background-save errors are ignored

Privacy/reporting preference setters discard SQLite errors
([portfolio_host.rs](../../src/host/portfolio_host.rs#L269)), and a completed history save is also
discarded ([portfolio_host.rs](../../src/host/portfolio_host.rs#L150)). The UI can therefore show a
choice that silently reverts after restart, or repeatedly rebuild history without explaining why.

Return/report these errors through the host's existing error state. Background failures should be
visible but non-destructive and should retain retry state.

## Positive observations

- Host mutations are copy-on-write and publish only after the SQLite transaction succeeds.
- Portfolio table reads are strict; undecodable transaction/snapshot rows are not silently dropped.
- Quote projections are generation checked and expensive currency work stays off the UI thread.
- Sync has thoughtful orphan handling, invalid-ledger holding, transaction-level records, and
  history invalidation.
- Pure formatting, layout, import, accounting, host, store, card, and sync code already has broad
  unit coverage; the architecture is testable without a window.
- Privacy formatting is consistently represented in the view input rather than mutating values.

## Required production gate

Before calling the feature production-ready:

1. Fix H1–H8 and add the regression tests described above.
2. Establish exact numeric and rounding rules for every transaction type and currency/asset scale.
3. Define one time model for daily snapshots, live points, prior-day comparison, local calendar
   boundaries, and FX observations.
4. Make history completeness explicit and retryable per asset; prove cross-currency and >1,000-day
   behavior with deterministic fake providers.
5. Add ledger-wide invariant validation on load, local mutation, import, and sync application.
6. Benchmark mutation/persistence and history rebuilds at realistic upper-bound dataset sizes.
7. Run the ignored portfolio galleries and a manual real-window pass in both themes, multiple scale
   factors, DST and non-DST time zones, and at least USD/EUR/BTC base currencies.

## Verification performed

- `cargo test --lib portfolio --no-fail-fast`: **passed** — 241 passed, 17 ignored, 5,132 filtered.
- `cargo fmt --all -- --check`: **passed**.
- `cargo test`: **passed** — 5,311 passed, 79 ignored, 0 failed (plus empty binary/doc-test
  targets).
- `cargo clippy --all-targets --all-features -- -D warnings`: **passed**.
- Full Pine corpus (`cargo test --features corpus`): not run; unrelated to portfolio behavior and
  intentionally excluded from the normal fast suite.

## Missing regression coverage exposed by this review

- A full close must retain realized P&L in statistics and every later snapshot.
- Decimal decomposition (`0.3 - 0.1 - 0.2`) must close exactly under the chosen numeric contract.
- Zero/negative/non-finite price and fee; empty/mismatched asset identity; nonexistent portfolio.
- Moving a transaction between portfolios when its source portfolio has dependent disposals.
- Same-day contribution after the final history bucket.
- Daily-bar close versus transaction cutoff; current forming daily bar.
- One failed asset among several, stale in-flight rebuild, and retry after partial success.
- Cross-currency daily history and history longer than 1,000 days.
- DST boundaries and editing a transaction in a different seasonal offset.
- Closed/delisted assets must not count as current unpriced exposure.
- Previous-day reporting-currency change when FX moves but the asset does not.
- Duplicate portfolio naming and CSV round-trip routing.
