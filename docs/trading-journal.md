# Trading journal

A journal for every execution DegenView knows about. Paper trading is the first source; brokers and
file imports are meant to arrive as further sources without touching the journal's core. Research
and the comparison with other journals: [trading-journal-research.md](reviews/trading-journal-research.md).

**Status.** Built: the canonical execution model, trade reconstruction, the paper source, storage
and capture, the pure analytics (statistics, equity and drawdown, calendar, filters, R-multiple,
Edge Score), and the Journal tab with the Overview, Calendar and Trades pages, the trade panel and
reviews. Breakdowns, playbooks, the daily journal, charts, import/export and iCloud follow as
separate changes; their sections are added as they land.

## Layers

```
paper snapshot ──► PaperSource ──► normalize ──► journal_execution (SQLite, append-only)
                                                        │
                                              reconstruct (pure) ──► trades ──► analytics, UI
```

- `model/journal/` is pure: no egui, no SQLite, no network. `model.rs` (records), `source.rs` (the
  source trait), `normalize.rs` (validation), `accounting.rs` (exact arithmetic), `reconstruct.rs`
  (executions to trades), `sources/paper.rs` (the paper adapter).
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

`reconstruct(executions, basis_of)` is a pure function.

1. Group by `(account, instrument key)`. Replay each group by `(time, input order)`; voided and
   non-positive executions are ignored. The instrument key is the source's own (the paper key, as
   the engine groups positions), not the normalised market key.
2. A trade opens on the first execution while flat and closes when flat again. Scaling in and out
   stays in one trade. An open position is an open trade.
3. A fill that crosses flat is split. The closing quantity ends the trade; the remainder opens the
   next one. The fee follows the quantity, the opening part taking the remainder so the parts add
   back to the whole fee. A trade's key is the uid of the execution that opened it.
4. Entry fees stay attached to the open quantity and are charged pro rata as it closes, as the
   engine does. `fees` is what has been charged so far; `open_fees` is still attached.
5. Gross P&L of a reduction follows the account's cost basis: `AverageCost` (paper: the engine keeps
   one running average), `Fifo` or `Lifo`. A closed trade's total is the same under all three; the
   leg that earned it differs.
6. A missing fee leaves `fees_known` false: the figure covers only the known fees and net P&L is
   not final.

A test runs real engine order sequences and checks that the journal's average-cost net P&L equals
the engine's closed trades and the account's `realized_pnl` once the position is flat.

## Analytics

Pure functions over trades in `model/journal/`: `analytics.rs` (summary), `equity.rs` (curve and
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

## The Journal tab

A top-level tab (`TabContent::Journal`, one per app, persisted like the Portfolio tab) opened from
the toolbar's Trade Journal button. `host/journal_host.rs` holds the app-wide state and
`host/journal_tab.rs` the per-window view; `ui/journal_table.rs` and `ui/journal_detail.rs` draw.

**Data flow.** `JournalHost` rebuilds the trades on a worker thread (`work_queue`) from the stored
executions whenever the paper ledger's revision changes, and publishes the result as an `Arc`
only once it is complete, so a frame never waits on it. Each frame the tab asks "did the ledger
revision change?" (a counter compare, no SQLite) and polls a channel. A stored execution that no
longer decodes fails the build and the last good trades stay on screen with the error; leaving the
row out would change a trade's result. The table's row order (filter then sort) is cached against
the build, the reviews, the filter and the sort and recomputed only when one changes.

**Header.** Range presets (Today, Week, Month, Year, YTD, All, Custom with two `YYYY-MM-DD` ends,
the end date included), account (all, or one; archived accounts are marked), instrument, side,
status, symbol search, reset, column visibility. Everything is one `TradeFilter`; the preference
(range, account, sort, visible columns and widths) is saved in the local `journal.prefs` setting
when the pointer is released, never while a column is being dragged.

**Table.** Sortable headers, drag-to-resize columns, virtualised rows (only visible rows are
formatted), arrow keys or `j`/`k` to move, Escape to close the panel. Columns: opened, closed,
symbol, account, side, status, quantity, average entry and exit, gross, fees, net, return, R,
duration, strategy, tags, review. Return is net P&L over the entry notional, not over margin or
equity. A fee total with an unknown part is shown as a floor (`>= 1.00`). Open trades show no exit
and no result until something has closed.

**Trade panel.** Summary, every execution with the position after it and the P&L it realised, and
the review. Previous and next follow the rows as filtered and sorted.

**Review.** Notes, strategy and setup (free text until playbooks exist), tags, mistakes, emotions,
a 1 to 5 rating, a reviewed switch, and the user's own stop and target. A stop entered here
overrides the one the source attached when R is computed. It saves 0.8 s after the last change,
when a field loses focus, and when another trade is opened; an empty review is not stored. A review
is keyed by the trade's key and lives in `journal_review`; it never changes a trade, an order or a
balance (tested). A trade whose executions disappear leaves its review behind.

**Resetting or deleting a paper account** asks whether to keep its history in the journal (a switch,
on by default, in the delete confirmation and in the reset sheet). Keeping archives the account;
removing deletes its executions and reviews. An MCP reset keeps.

## Overview and Calendar pages

The tab has three pages (`Section`, remembered in `journal.prefs`): Overview, Calendar, Trades.
The header (range, account, instrument, side, status, search) filters all of them; the Calendar
ignores the range, because it has its own month navigation.

**Overview** (`model/journal/overview.rs` computes it, `ui/journal_overview.rs` draws it): five
headline cards (net P&L, win rate, profit factor, average win/loss, day win rate, each with a
tooltip definition and, where the range has a previous period, the change against it), twelve
smaller figures (gross, fees, trades, expectancy, average R, max drawdown, trading, winning, best
and worst day, duration, streak), the realised-equity curve with an optional previous-period
overlay, the Edge Score radar, daily P&L bars, drawdown (amount or percentage, with the overlay),
the distribution of results, long against short, and the latest closed trades (click opens one).
A figure that does not exist is a dash with the reason as its tooltip, never a zero.

- **One currency at a time.** A selection that spans currencies (a USD and a USDT account) shows
  chips to pick which one the page describes; amounts are never added.
- **Equity needs a balance.** With every account's initial balance known the curve is *realised
  equity* (balance, plus what was earned before the range, plus the range's trades) and drawdown
  has a percentage; otherwise it is titled *cumulative net P&L* and says it is not account equity.
- **Previous period.** The window of equal length just before the range, shifted onto it; it has
  no previous period for "All" or a half-open custom range.
- **Distribution.** Bins of a round width whose edges are multiples of it, so zero is always an
  edge and a bar is wholly a win or wholly a loss.
- Computed when the filter, the build or the currency changes, not per frame.

**Calendar** (`ui/journal_calendar.rs`): one tile per day tinted by its result with the net, an
arrow, the trade count and win rate; a week total at the end of each row (Sunday-first); month net,
best and worst day and activity underneath; hover for the day's tally; click opens that day on the
Trades page. Sign is always in the text and an arrow, so colour is never the only cue.

Not built yet: breakdowns by hour, weekday, instrument and strategy, account Compare, playbooks and
the daily journal, chart markers and "view on chart", CSV import and export, iCloud sync of
reviews. There is no drag-to-zoom on the charts: the range presets are the zoom.
