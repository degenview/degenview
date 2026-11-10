# Trading journal

A journal for every execution DegenView knows about. Paper trading is the first source; brokers and
file imports are meant to arrive as further sources without touching the journal's core. Research
and the comparison with other journals: [trading-journal-research.md](reviews/trading-journal-research.md).

**Status.** Built: the canonical execution model, trade reconstruction, the paper source, storage
and capture, and the pure analytics (statistics, equity and drawdown, calendar, filters, R-multiple,
Edge Score). The Journal tab, reviews, charts, import/export and iCloud follow as separate changes;
their sections are added as they land.

## Layers

```
paper snapshot ──► PaperSource ──► normalize ──► journal_execution (SQLite, append-only)
                                                        │
                                              reconstruct (pure) ──► trades ──► analytics, UI
```

- The pure domain layer is the [`degenview-journal`](https://github.com/degenview/journal) crate (its own repository, published to crates.io and pinned
  to an exact version): `model` (records), `source` (the source trait), `normalize` (validation),
  `accounting` (exact arithmetic), `reconstruct` (executions to trades), analytics, reviews, CSV and
  backup. It has no egui, SQLite, network, paper or iCloud code and knows no provider. This
  repository holds what is DegenView's: the paper **connector** (`model/journal_paper_source.rs`), the
  provider catalog for imports (`model/journal_markets.rs`), storage, host, UI and sync.
- `store/journal.rs` owns the tables and the ingest. Nothing in it touches a `paper_*` table.
- A source only reports what happened. `TradingActivitySource` has no method that places or changes
  an order, so a read-only broker connection needs no trading permission and no journal action can
  reach a market. The journal is handed an immutable `PaperTradingSnapshot`, never the host or the
  engine.

## Executions

An `Execution` is one fill, normalised: positive quantity and price, an optional fee (an absent fee
is not a zero fee), contract multiplier, UTC millisecond time, the order it came from, and for paper
the protective stop that was attached to the entry order. Money is `rust_decimal::Decimal`. The
paper engine's `f64` becomes the decimal it prints as (`0.1` is exactly `0.1`), so sums over a long
history do not drift. Values that cannot be one (NaN, infinity, out of range, not positive) are
rejected with a reason in the ingest report, never stored and never dropped silently.

### Identity

- The journal's key for a fill is `uid`, stable across devices and lifetimes. For paper it is
  `paper:<account created_at>:<fill timestamp>:<k>`, where `k` counts the account's fills with the
  same timestamp in ledger order. The engine's own fill id is kept in `source_exec_id`: it is a
  per-Mac counter that is reused after a reset and tagged when it syncs, so it names nothing
  durable.
- A paper account is `paper:<created_at>`. Resetting an account gives it a new `created_at`, so the
  journal sees a new account and the old one's history stays under it, marked archived. Two
  accounts made in the same millisecond get a `-1`, `-2` suffix in id order.
- Storage enforces `UNIQUE(uid)` and `UNIQUE(account_uid, source_exec_id)`; ingest is
  `INSERT OR IGNORE`. Replaying a source never duplicates, and two separate fills with the same
  time, price and size stay separate because their identities differ.

### Capture

`Store::save_paper_trading` calls `journal_ingest_paper` inside its own transaction, so a fill and
its journal row commit together whoever caused the fill: the ticket, the tables, the chart overlay,
MCP, or a quote crossing a working order. Only the new fills are read (a per-account cursor). The
engine is not hooked: `PaperHost` evaluates throwaway candidate engines for order checks, so a
callback there would fire for fills that never happened.

`journal_reconcile_paper` runs the same insert over the whole snapshot with no cursor. It runs when
`PaperHost` loads (history that predates the journal, a crash between passes) and the cursor-based
ingest runs again on `reload` (fills another Mac made, which arrive through the iCloud bundle
rather than through this Mac's save). Executions are **not** synced separately; they are rebuilt on
each Mac from the bundle that carries the ledger.

A journal failure never fails a paper save. The journal part rolls back on its own, the reason is
kept in `journal_meta.last_error`, and the next pass retries.

### Reset and delete

A paper account that is reset or deleted keeps its journal history: the account is marked archived
and still counts. `Store::journal_purge_account` removes one account's journal data for good; the
paper UI asks which to do before it resets or deletes (see below, once the tab lands). MCP-initiated
resets cannot ask and keep the history.

## Trade reconstruction

Pure and in the [`degenview-journal`](https://github.com/degenview/journal) crate: a trade is flat to flat, a fill that crosses flat is split with its
fee, entry fees are charged pro rata as the position closes, and gross P&L follows the account's cost
basis (average cost, FIFO or LIFO). The rules and the worked cases are in the crate's
`docs/model.md`. What stays here is the check against the paper engine: a test runs real engine order
sequences and asserts that the journal's average-cost net P&L equals the engine's closed trades and the
account's `realized_pnl` once the position is flat (`model/journal_paper_source.rs`).
## Analytics

Statistics, realised equity and drawdown, the calendar, filters, the R-multiple, Edge Score v2 and the
Process score are pure functions in the [`degenview-journal`](https://github.com/degenview/journal) crate; every figure that may not exist is a
`Metric` with a reason. Their definitions (win rate, profit factor, expectancy, streaks, days and
ranges, R, the two places the Edge Score follows LuxAlgo's documentation rather than its code) are in
the crate's `docs/model.md`. This repository only draws them.
# Trading journal

A journal for every execution DegenView knows about. Paper trading is the first source; brokers and
file imports are meant to arrive as further sources without touching the journal's core. Research
and the comparison with other journals: [trading-journal-research.md](reviews/trading-journal-research.md).

**Status.** Built: the canonical execution model, trade reconstruction, the paper source, storage
and capture, and the pure analytics (statistics, equity and drawdown, calendar, filters, R-multiple,
Edge Score). The Journal tab, reviews, charts, import/export and iCloud follow as separate changes;
their sections are added as they land.

## Layers

```
paper snapshot ──► PaperSource ──► normalize ──► journal_execution (SQLite, append-only)
                                                        │
                                              reconstruct (pure) ──► trades ──► analytics, UI
```

- The pure domain layer is the [`degenview-journal`](https://github.com/degenview/journal) crate (its own repository, published to crates.io and pinned
  to an exact version): `model` (records), `source` (the source trait), `normalize` (validation),
  `accounting` (exact arithmetic), `reconstruct` (executions to trades), analytics, reviews, CSV and
  backup. It has no egui, SQLite, network, paper or iCloud code and knows no provider. This
  repository holds what is DegenView's: the paper **connector** (`model/journal_paper_source.rs`), the
  provider catalog for imports (`model/journal_markets.rs`), storage, host, UI and sync.
- `store/journal.rs` owns the tables and the ingest. Nothing in it touches a `paper_*` table.
- A source only reports what happened. `TradingActivitySource` has no method that places or changes
  an order, so a read-only broker connection needs no trading permission and no journal action can
  reach a market. The journal is handed an immutable `PaperTradingSnapshot`, never the host or the
  engine.

## Executions

An `Execution` is one fill, normalised: positive quantity and price, an optional fee (an absent fee
is not a zero fee), contract multiplier, UTC millisecond time, the order it came from, and for paper
the protective stop that was attached to the entry order. Money is `rust_decimal::Decimal`. The
paper engine's `f64` becomes the decimal it prints as (`0.1` is exactly `0.1`), so sums over a long
history do not drift. Values that cannot be one (NaN, infinity, out of range, not positive) are
rejected with a reason in the ingest report, never stored and never dropped silently.

### Identity

- The journal's key for a fill is `uid`, stable across devices and lifetimes. For paper it is
  `paper:<account created_at>:<fill timestamp>:<k>`, where `k` counts the account's fills with the
  same timestamp in ledger order. The engine's own fill id is kept in `source_exec_id`: it is a
  per-Mac counter that is reused after a reset and tagged when it syncs, so it names nothing
  durable.
- A paper account is `paper:<created_at>`. Resetting an account gives it a new `created_at`, so the
  journal sees a new account and the old one's history stays under it, marked archived. Two
  accounts made in the same millisecond get a `-1`, `-2` suffix in id order.
- Storage enforces `UNIQUE(uid)` and `UNIQUE(account_uid, source_exec_id)`; ingest is
  `INSERT OR IGNORE`. Replaying a source never duplicates, and two separate fills with the same
  time, price and size stay separate because their identities differ.

### Capture

`Store::save_paper_trading` calls `journal_ingest_paper` inside its own transaction, so a fill and
its journal row commit together whoever caused the fill: the ticket, the tables, the chart overlay,
MCP, or a quote crossing a working order. Only the new fills are read (a per-account cursor). The
engine is not hooked: `PaperHost` evaluates throwaway candidate engines for order checks, so a
callback there would fire for fills that never happened.

`journal_reconcile_paper` runs the same insert over the whole snapshot with no cursor. It runs when
`PaperHost` loads (history that predates the journal, a crash between passes) and the cursor-based
ingest runs again on `reload` (fills another Mac made, which arrive through the iCloud bundle
rather than through this Mac's save). Executions are **not** synced separately; they are rebuilt on
each Mac from the bundle that carries the ledger.

A journal failure never fails a paper save. The journal part rolls back on its own, the reason is
kept in `journal_meta.last_error`, and the next pass retries.

### Reset and delete

A paper account that is reset or deleted keeps its journal history: the account is marked archived
and still counts. `Store::journal_purge_account` removes one account's journal data for good; the
paper UI asks which to do before it resets or deletes (see below, once the tab lands). MCP-initiated
resets cannot ask and keep the history.

## Trade reconstruction

Pure and in the [`degenview-journal`](https://github.com/degenview/journal) crate: a trade is flat to flat, a fill that crosses flat is split with its
fee, entry fees are charged pro rata as the position closes, and gross P&L follows the account's cost
basis (average cost, FIFO or LIFO). The rules and the worked cases are in the crate's
`docs/model.md`. What stays here is the check against the paper engine: a test runs real engine order
sequences and asserts that the journal's average-cost net P&L equals the engine's closed trades and the
account's `realized_pnl` once the position is flat (`model/journal_paper_source.rs`).
## Analytics

Pure functions over trades in the [`degenview-journal`](https://github.com/degenview/journal) crate: `analytics.rs` (summary), `equity.rs` (curve and
drawdown), `calendar.rs` (days and the month view), `filters.rs` (selection and ranges), `risk.rs`
(R), `edge.rs` (Edge Score), `metric.rs` (the figure-or-reason type). None reads egui or SQLite.

### A figure that may not exist

Every statistic is a `Metric<T>`: a value, or the `Reason` there is none (no trades, no losses, no
stop, no initial balance, mixed currencies, too few samples, no spread). Nothing is NaN or
infinite: a profit factor with no losses is "no losses", an empty selection is "no closed trades"
and not zero, and a test runs degenerate records and checks every ratio is finite.

### Definitions

- **Closed trade**: flat to flat. Open trades are counted separately and are in no realised figure.
- **Outcome**: net P&L above `+tolerance` wins, below `-tolerance` loses, otherwise breakeven. The
  tolerance is an amount in the account currency (default zero).
- **Win rate** = wins / closed trades. **Gross profit / gross loss**: the sum of positive and of
  negative net P&L of all closed trades, whatever their label. **Profit factor** = gross profit /
  gross loss. **Average win / loss**: mean net P&L of winners / losers; **payoff** is their ratio.
  **Expectancy** = net P&L per closed trade.
- **Streaks** run in close order; a breakeven ends both.
- **Equity** is *realised*: cumulative net P&L of closed trades at each close, plus the initial
  balance when known. It is not account equity (no unrealised P&L, and paper accounts have no cash
  movements). **Drawdown** is the distance below the running peak of cumulative P&L, which starts at
  zero; its percentage is `drawdown / (initial balance + peak)` and exists only with a positive
  initial balance.
- **Sharpe, Sortino, Calmar** use daily returns on trading days (days with no closed trade are not
  zero-return days), are not annualised, take a zero risk-free rate, and need an initial balance and
  at least 30 trading days. **SQN** is `sqrt(n) x mean R / stdev R` and needs 30 trades with an R.
  Below the thresholds they are unavailable with the count, never a guess.
- **Currencies** are never added: a selection with more than one currency is refused by
  `summarize` and split by `summarize_by_currency`. There is no conversion.

### Days and ranges

A trade belongs to the day it **closed**, in the journal's time zone, at the offset in force at the
closing instant, so a daylight-saving day is 23 or 25 hours. A trade that spans midnight counts on
its close day; open trades belong to no day. The calendar, daily P&L, day statistics and the
weekday breakdown all use this rule. A range filter uses the same: a closed trade is in
`[from, to)` by its close time, an open one by its open time. Presets: Today, Week (starts Sunday
or Monday), Month, Year (the last 365 local days), Year to date, All; the previous period is the
window of the same length just before.

### R-multiple

`initial risk = sum over entry legs of quantity x |entry price - stop| x multiplier`, with the
stop the user entered, else the one the source attached to the entry order, else none: a stop is
never guessed, and a stop on the wrong side of an entry is invalid, not negative risk. Scaling in
therefore adds the risk taken at each entry. **Realised R** = net P&L / initial risk, closed
trades only.

### Edge Score (version 2)

The published LuxAlgo composite, reimplemented natively from `LuxAlgo/trade-journal`
(`packages/core/src/edge-score.ts`, `docs/edge-score.md`; the repository states the MIT licence).
Six components against full-marks thresholds, weights summing to 100: win rate 60% (15), profit
factor 3.0 (25), average win over average loss 2.5 (20), drawdown 0% of balance plus peak and zero
at 25% (15), recovery factor 3 (10), consistency with the largest winning day at most 15% of
winning-day profit (15). Withheld under five closed trades, rounded to two decimals, shown with the
trade count.

The published code and documentation disagree in two cases; this follows the **documentation**:
no drawdown at all with a known balance scores 100 (the reference code gives its neutral 50), and
no initial balance scores a neutral 50 (the reference code measures against peak P&L). So results
match LuxAlgo's except in those two cases. The score uses only objective trade results; review
ratings never enter it. A separate process score (rule adherence, checklists) will sit beside it
and never blend with it.
