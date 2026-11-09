# Trading journal

A journal for every execution DegenView knows about. Paper trading is the first source; brokers and
file imports are meant to arrive as further sources without touching the journal's core. Research
and the comparison with other journals: [trading-journal-research.md](reviews/trading-journal-research.md).

**Status.** The foundation (this page's first sections) is built: the canonical execution model,
trade reconstruction, the paper source, storage and capture. The analytics, the Journal tab,
reviews, charts, import/export and iCloud follow as separate changes; the sections for them are
added as they land.

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
