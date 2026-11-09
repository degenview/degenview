# Trading journal research

What the journal copies, borrows and deliberately leaves out, with the evidence for each claim.
Read 2026-11-08 and 2026-11-09 from the products' own pages. **D** = documented on an official
docs or repository page, **M** = marketing copy only, **NF** = not found on the pages read,
**U** = could not be verified. "Not found" means not on the pages read, not absent from the
product.

## LuxAlgo Journal

Sources: `docs.luxalgo.com/platform/workspaces/journal` (overview, dashboard, accounts, trades,
breakdown), `luxalgo.com/trade-journal`, `github.com/LuxAlgo/trade-journal` (MIT) and its
`docs/edge-score.md`. Not read: the Notes and Compare doc pages, `docs/importers.md`,
`docs/market-data.md`, `packages/core/src/edge-score.ts`.

| Area | Finding |
| --- | --- |
| Placement | D: on the account, not in a workspace. A column beside the chart that maximises into a Journal tab (Dashboard, Breakdown, Trades, Notes, Compare); "Return to chart" restores it. |
| Chart markers | D: for the active chart's symbol and the selected account, an entry triangle, an exit flag and a dashed line carrying the net P&L. Not drawings, not saved. "Show on chart" loads the symbol and frames the hold. |
| Dashboard | D: range tabs (Today, Week, Month, Year, YTD, All, custom); cards for net P&L, win rate, profit factor, average win/loss and day win rate with a delta against the previous window; performance curve with a previous-period overlay; daily P&L; drawdown (percent only with an initial balance); calendar with week and month totals; risk block (max drawdown, expectancy, average duration, streak, best and worst day). M: a win rate in each calendar cell. |
| Trades | D: the log is fills; round trips are derived flat to flat; status Open, Win, Loss or Breakeven; review has a 1 to 5 rating, a reviewed flag, stop, target, tags, mistakes and notes; running P&L per fill. R needs a stop. |
| Breakdown | D: day of week (by close), entry hour, hold time, symbol, side, position size quartile, tag, rating; the written read needs three trades in a slice. |
| Accounts | D: broker sync, file import (CSV, TXT, HTML to 2 MB) or manual; cost basis FIFO (default), LIFO or weighted average; paper accounts appear as journal accounts; deleting an account removes its fills and reviews. |
| Edge Score | D (repository, version 2): withheld under 5 closed trades; win rate (full marks at 60%, weight 15), profit factor (3.0, 25), average win over average loss (2.5 to 1, 20), drawdown (0% full, 25% or more zero, 15, neutral 50 without an initial balance), recovery factor (net 3 times max drawdown, 10), consistency (largest winning day at most 15% of day profits, 15); score is the weighted mean. U: the curve between zero and full marks; read `edge-score.ts` before implementing. |
| Import / export | D: about a dozen statement formats and read-only broker sync; re-import skips fills already held. Export is documented only for the open-source edition (trades CSV, records JSON; no general JSON restore). NF: export on the hosted product. |

Look and feel (screenshots on the product page, dark theme only): a slim header with range tabs and
an account picker, a left rail of sections, five KPI cards with a ring or split bar, a smooth
equity line with a soft fill beside the Edge Score radar, daily P&L bars beside a red drawdown
area, a calendar of tinted tiles (date, net P&L, `3t · 33%`) with a month footer, a trade table
with LONG/SHORT and WIN/LOSS pills, and chart markers joined by a dashed line ending in a filled
P&L chip. No light-theme capture exists.

## Competitors

| Product | Documented | Marketing or unverified |
| --- | --- | --- |
| TradeZella | Playbooks with rules; trade replay (jump to an execution); daily journal and templates; tags for setups, emotions and mistakes; multiple take-profit and stop-loss, running P&L, MAE/MFE; 50+ reports including tag performance and expectancy; broken rules flagged. Zella Score is 0 to 100 over profitability, risk management, consistency and discipline; **weights NF**. | "500+ brokers", prop-firm sync, AI agents (the page shows sample data). |
| TraderSync | **U**: the site answered HTTP 403 and nothing was read. | everything |
| TradesViz | A trade is a group of executions; imports de-duplicate on timestamp plus symbol; pivot grid with cross-filtered charts; custom dashboards; MFE/MAE and exit-efficiency columns; a rules-based (not LLM) coach gated by a two-proportion test with at least 10 trades per detector; REST and MCP with per-scope permissions that "cannot place orders". | statistic counts disagree between pages (80+, 150+, 600+). |
| Edgewonk | Tiltmeter over the equity curve (**no published formula**); mistake and behaviour tags; Efficiency as a percentage of rule-following; weekly Edge Finder; Chart Lab reports (Sharpe, Sortino, profit factor, Calmar, gain to pain, SQN, drawdown depth and length, exit analysis, time buckets); missed trades; daily, weekly and monthly review sessions. | "millions of reviewed trades", simulators. |

## What DegenView takes

From LuxAlgo: the round-trip model, account picker over every page, the dashboard set, calendar,
chart markers, Edge Score version 2 (credited, versioned, withheld under five trades), the three
cost bases. From TradeZella and Edgewonk: playbooks with rule adherence, mistake and emotion tags
reported as associations, a process score kept apart from the Edge Score. From TradesViz and
Edgewonk: candle-derived MFE/MAE labelled as an estimate, exit efficiency, Sharpe, Sortino, Calmar
and SQN only with enough samples, and a sample-size gate on every breakdown.

Not claimed: parity with any product, anything about TraderSync, or the formulas behind the Zella
Score and the Tiltmeter, which are not published.
