# Trading journal

A journal for every execution DegenView knows about. Paper trading is the first source; brokers and
file imports are meant to arrive as further sources without touching the journal's core. Research
and the comparison with other journals: [trading-journal-research.md](reviews/trading-journal-research.md).

**Status.** Built: the canonical execution model, trade reconstruction, the paper source, storage
and capture, the pure analytics (statistics, equity and drawdown, calendar, filters, R-multiple,
Edge Score), and the Journal tab with the Overview, Calendar, Trades, Analytics and Compare pages,
Playbooks and Daily pages, the trade panel and reviews. Screenshots and MFE/MAE, charts,
import/export and iCloud follow as separate changes; their sections are added as they land.

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

**Overview** (`degenview_journal::overview` computes it, `ui/journal_overview.rs` draws it): five
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

## Analytics and Compare pages

**Analytics** (`degenview_journal::breakdown`, `ui/journal_analytics.rs`). A risk and consistency
block (max, average and current drawdown, episodes, Sharpe, Sortino, Calmar, SQN, average, best and
worst R, streaks, how long winners and losers are held), the distribution of R, and a breakdown of
results by one dimension at a time with a bar chart and a table (trades, win rate, profit factor,
average R, net, sample flag).

Dimensions: weekday (by the day the trade *closed*), hour (the hour it *opened*), instrument, side,
hold time (under 1 min up to over 7 days), size (the entry notional in quartiles of the current
selection, so the buckets are relative to it), and from the review: strategy, tag, mistake,
emotion, rating. Tags, mistakes and emotions **overlap**: a trade with two tags is in both slices,
so counts add up to more than the number of trades; the page says so. Slices from reviews put
trades with no value in "(none)", always last.

**Sample size.** A slice with fewer than 5 trades is marked THIN and fewer than 30 SMALL, with the
reason in a tooltip. Nothing on the page claims a difference is significant, and associations with
tags, mistakes and emotions are descriptive, not causal.

**R** uses the stop on the trade or, when the review has one, the stop the user entered.

**Compare** (`degenview_journal::compare`, `ui/journal_compare.rs`): accounts side by side over the
same range and filters, in one currency (others are left out, never converted). A cumulative net
P&L overlay that starts every account at zero, so it compares what each earned and how it moved
rather than how large it is, and a table of closed trades, net, win rate, profit factor,
expectancy, max drawdown, average R, day win rate and Edge Score per account. It needs two
accounts with trades in the selection and says so otherwise. Percentages need each account's own
initial balance.

## Playbooks, the daily journal and the review workflow

**Reviews** now hold: a playbook (by id) with its checklist ticks, setup, entry and exit rationale,
tags, mistakes, emotions (one-click suggestions: a default set plus everything already used),
1 to 5 discipline and confidence ratings, a planned entry, stop and target, notes, the star rating
and the reviewed switch. The plan read-out shows the **planned reward to risk** (target distance
over stop distance from the planned entry, else the fill; never without a target and a valid stop)
and the **risk as a share of the account's starting balance** (needs a balance). A review's
strategy is its playbook: the Strategy column, sort, breakdown and filter all use the playbook's
name.

**Playbooks page** (`degenview_journal::playbook`, `ui/journal_playbooks.rs`). A playbook has a name,
description, setup criteria, entry, exit and risk rules, a checklist (one rule per line) and tags.
The page compares every playbook over the current range and filters: trades, win rate, profit
factor, expectancy, average R, net, THIN/SMALL flags, and **rule adherence**, the mean share of its
checklist ticked over its reviewed trades (a checklist item is matched by its text). Edits save
after a pause, on blur and when you leave the page; deleting asks first and leaves reviews'
other notes intact.

**Daily page** (`degenview_journal::daily`, `ui/journal_daily.rs`). One entry per local calendar day for
the whole journal: outlook, plan, goals, emotional state, session notes, mistakes, lessons and an
end-of-day review, next to that day's closed trades (a click opens one) and their tally. Days with
no trades are fine, nothing is required, and an entry with nothing in it is not stored. Clicking a
calendar day opens it here.

**Psychology.** Emotions and mistakes are free tags with suggestions; the Analytics breakdown by
mistake or emotion shows outcomes per tag and says the associations are descriptive.

**Process score** (`degenview_journal::process`), shown beside the Edge Score and never blended into
it: rule adherence (40), discipline (30: the mean 1 to 5 rating scaled to 0 to 100) and clean
execution (30: reviewed trades with no mistake noted), over the *reviewed* closed trades. A
component with no inputs is left out and the weights of the rest renormalised; withheld below 10
reviewed trades. It records self-reported process, not skill, and a rating never changes an
objective figure. The Analytics page also lists each Edge Score component with its value, weight,
input and full-marks threshold.

## Screenshots and excursion

**Screenshots** (`host/journal_shots.rs`, `ui/journal_shots.rs`). A review holds up to 12 pictures,
attached from a file (the system chooser), pasted, or dropped on the window while a trade is open.
They go through `image_ingest` like drawing pictures (PNG or JPEG, shrunk to at most 600 kB, no
upload anywhere) into the existing content-addressed asset store (`drawing_asset`), and the review
keeps only their hashes (`Review.images`). Reading and storing run on a worker; thumbnails get
their texture once the asset host has decoded the picture; a click opens a full-size viewer.
Attach and remove save the review at once. Removing a picture from a review leaves it in the store,
and removing an account's journal history leaves its pictures too: the store is shared with
drawings, never garbage-collected, and syncs by hash already. Capturing the chart as a screenshot
comes with the chart integration.

**MFE / MAE** (`degenview_journal::excursion`, `host/journal_excursion.rs`). How far a closed trade
went for and against the trader, **estimated** from candles and labelled so ("Estimated from N 5m
candles between the fills"). The finest candle interval whose bars over the hold fit in one
request (1m up to 1d, at most 1,000 bars) is fetched from the trade's own provider. Rules:

- A bar containing a fill is never used for its high or low: whether the extreme came before or
  after the fill is unknown. Fills themselves are points on the path, valued at the fill price.
- Only bars lying wholly between the first and last fill count. With none, the answer is
  "unavailable" with the reason, never a number built from the fills alone.
- The position is valued at its weighted-average cost plus what earlier exits realised, so scaling
  in and out is followed. Gross of fees. MFE and MAE are floored at zero; exit efficiency is gross
  result over MFE (negative when a profitable stretch ended in a loss, none when it never was).
- A fetch that fails or returns nothing is shown with Try again and not cached. A figure, and the
  deterministic "no complete bar fits", are cached in the rebuildable `journal_trade_stats` table
  keyed by the fills and interval, so editing history re-estimates.

Open trades show no block. Markets without a candle provider (imports) say so. The estimate starts
when a closed trade is opened, once per session per set of fills.

Paper accounts now get a creation time no other account shares (the engine bumps it by a
millisecond when needed): the journal names an account by it, and two accounts made in one
millisecond used to swap names when one was deleted.

## Trades on charts and View on chart

**Markers** (`degenview_journal::marks`, `ui/journal_markers.rs`, `ui/chart_card/journal_marks.rs`).
Every market chart draws the journal's trades for its own market, in the vocabulary of LuxAlgo's
journal: a **triangle at each entry fill** (up for a buy, down for a sell), a **diamond at each exit**,
a **dashed line** between the average entry and the average exit in the result's colour, and a
**chip with the net result** (`+112.88`). Win and loss use the chart's own up and down colours; the
triangle's direction and the chip's sign carry the result as well, so colour is never the only cue;
breakeven and open trades are neutral grey and an open trade has no line or chip.

- Prices are the fills' own, never a constructed bar's, so a mark sits where the trade happened
  under every chart type. Time uses the same time-to-x map as drawings, so a trade shorter than a
  bar sits inside it. Marks never feed the price scale: one far off the range is clipped to the
  plot's edge, not chased (a test pins it).
- Drawn in the shared frame, so a **snapshot** includes them only when it includes trading
  (`snapshot.prefs`, off by default), and in a **replay** a trade that has not happened yet at the
  cursor is left out.
- Clutter: past 120 trades in view only the glyphs are drawn (no lines or chips); a chip that would
  sit on another is dropped; at most 600 trades are laid out per frame.
- **Hover** shows a tooltip (market, side, result, entry to exit, quantity, net) and a pointing
  hand; **a click opens the trade in the journal** (`AppAction::OpenJournalTrade`: the range and
  filters that could hide it are cleared and its panel opens). The click is ignored while a drawing
  tool is active.
- A chart setting, Canvas ▸ **Journal trades** (on by default; `canvas.show_journal_trades`).
  The workspace hands each chart its market's marks from the journal host each frame (a pointer
  copy); the journal is read only while some chart has the setting on. Which account's trades show
  follows the account picker of the journal tab (all accounts when it is on All).

**View on chart.** The trade panel's button asks the app to open the trade's market on a chart,
framed on the hold. A chart tab that already charts the market is used (a new tab otherwise). The
workspace timeframe is kept when it shows the hold in 2 to 150 bars, else it is changed to the
smallest that fits in 80 bars (never below 1H) and the chart says so. If the candles held do not
reach the trade's first fill the chart goes to its oldest bar, which makes it page older history in
by itself, and frames the trade when it arrives; if the provider has nothing older (or serves no
older history, or 30 s pass) it shows what it has and says that the start of the trade is off the
left edge. Nothing is fabricated. Charts are 1H or coarser, so a trade shorter than a bar is one
mark inside a bar.

## Import, export and backup

The Data menu in the journal header.

**Import executions (CSV)** (`degenview_journal::csv`, `ui/journal_import.rs`, `host/journal_io.rs`).
One execution per row. Required columns: time, symbol, side, quantity, price. Optional: account,
execution id, chart provider, fee, fee currency, contract multiplier, asset class, currency, order
id. Columns are found by their usual header names (`Date`, `Ticker`, `Action`, `Shares`, `Fill Price`,
`Commission`, `Exchange`, ...) and can be remapped in the sheet; a column serves one field.

- Every row either becomes a validated execution or is **refused with its row number and the
  reason**; the sheet previews the first rows, the counts and every refusal before anything is
  written, and the import runs on a worker. Cancelling writes nothing.
- Times: ISO-8601 with a zone, epoch seconds or milliseconds, or a clock time with no zone, which is
  read in UTC or this Mac's zone *as the user chooses* (never guessed). Numbers accept thousands
  separators and a currency sign. Side accepts buy/sell/b/s/bot/sld/long/short.
- **A blank fee stays unknown**, not zero; a negative fee is read as its size. A row whose currency
  column differs from the account's is refused (no cross-currency summing).
- **Identity and repeats.** A file's own execution id is the identity; without one, a hash of the
  fill plus how many identical fills came before it in the file, so two real identical fills stay
  two and importing the same file again adds nothing ("already in the journal"). A file id used
  twice in one file refuses the second row.
- The account is the file's account column, else the name typed in the sheet (default: the file's
  name), with a currency, an optional starting balance and a cost basis (Average, FIFO, LIFO; the
  paper engine is average cost). Each imported account is its own source, so importing another file
  never archives an earlier one. The optional **chart provider** links the trades to charts (markers,
  View on chart, MFE/MAE); without one they are journal-only and say so.
- Remove imported account (same menu) deletes the account's executions, trades and reviews after a
  confirmation; screenshots stay in the shared asset store.

**Exports.** Full-precision decimals, ISO-8601 UTC times and an explicit currency. *Trades* and
*Summary* are what the current range and filters show; *Executions* are the chosen account's (all
accounts on All), in the import format, so an export can be imported back (a test round-trips it to the
same trades, fees and unknown fees). *Notes* is a Markdown file of the daily entries and of every
trade review that has words, with tags and playbook. The summary has one row per figure per
currency with a `note` saying why a figure does not exist. Trades also carry the R multiple, hold
time, rating, tags, mistakes, emotions and notes.

**Backup and restore** (`degenview_journal::backup`, `Store::journal_backup` / `journal_restore`). A
versioned JSON file (`degenview-journal-backup`, version 1) with accounts, executions, reviews,
playbooks, daily entries and the screenshots reviews use. Trades and statistics are rebuilt from the
executions. Restore is a merge in one transaction: it adds what is missing, **never deletes, and
never overwrites a review, playbook or day the journal edited after the backup's copy** (it says how
many it left). Restoring the same file twice changes nothing. A file that is not a journal backup,
or is from a newer version, is refused with a reason; unusable parts (an invalid execution, one
whose account is not in the file, a damaged picture) are listed and the rest restored.

Not built, by decision: an embedded chart inside the trade panel and a "capture chart" screenshot
(attaching a picture from a file or the clipboard works; the chart's own camera menu already makes
snapshots).

## iCloud

Reviews, daily entries and playbooks sync (category **Trading journal**); executions, accounts,
trades and statistics do not (they are rebuilt per Mac from the paper bundle and imports). The
design, merge rule (last writer, with the loser's text kept), validation and what is tested are in
[iCloud Sync](icloud-sync.md#trading-journal). The three user tables are document tables, written
through `put_journal_document` / `delete_journal_document` (one row, one savepoint, upload queued in
it); a review is named by the hash of its trade key (`review_row_id`).
