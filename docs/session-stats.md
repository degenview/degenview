# Session statistics

"How often did this happen on the days this chart covers?", answered with the sample size and a
95% interval. A docked panel on the right of the chart (right-click → *Session statistics*) and
the read-only MCP tool `degenview_get_session_stats` ask the same engine.

The idea, the shape of the answer (`P(outcome | conditions)` with `N` and a confidence interval on
every number, a stability split, a row per year) and the outcome names come from LuxAlgo's
[edge-stats](https://github.com/LuxAlgo/edge-stats) (MIT). This is an independent implementation
over the bars the chart already holds: no code was copied, nothing from PineTS (AGPL) was used,
and nothing is fetched. The Wilson interval is checked against edge-stats' own published example
(24 of 30 is 62.7%–90.5%).

**These are historical frequencies, not predictions.** The panel says so under every answer, and so
does the MCP result's `disclaimer`. A condition picked after looking at the data overstates what it
finds, and a small sample says little; below 30 qualifying days the panel says the interval is wide.

## Where things live

| Piece | File |
| --- | --- |
| The engine: sessions, outcomes, filters, Wilson interval, halves, per-year, timing | `model/session_stats.rs` (pure, tests in `model/session_stats/tests.rs`) |
| The panel | `ui/session_stats_view.rs` (state, header, question, conditions), `session_stats_view/result.rs` (the answer cards), `parts.rs` (painted rows and bars), `text.rs` (numbers, times, the copied text) |
| Opening it | `Command::ToggleSessionStats` (`model/chart_context.rs`), `workspace/context.rs` (`show_stats_panel`), docked in `workspace.rs` |
| The MCP tool | `mcp/session_stats_tools.rs` (arguments), `mcp/session_stats.rs` (result), `host/mcp_host.rs::session_statistics` |

## The panel

A header (what it is of, a copy button, close), the question with a line on when a day counts, the
conditions, then the answer as cards: the estimate (a big number, the interval on a fixed 0–100%
axis, days counted, hits, the 95% interval), older against newer half, when in the day it
happened (1H only), a row per year, and callouts for what to know about the bars. Below 30
qualifying days the estimate carries a "Small sample" badge. Nothing qualifying is its own state
with a way to clear the conditions; a question the timeframe cannot answer says which timeframe.

- The conditions are seven weekday toggles, gap side, gap size in percent (a number field with a
  red outline and a hint while the text is not a number) and the previous day's direction. "Clear"
  appears once any is set.
- **Copy** puts the answer on the clipboard as plain text (`text::summary`): the question, the
  number with its sample and interval, the halves, the span and the reminder. It is the only thing
  the panel writes anywhere.
- Bars of a group (the halves, the years) share one text column, so equal shares draw equal bars.
- The weekday toggles, the number fields and every row carry accessibility labels; the toggles
  report their on/off state.
- The panel lays out from 280 to 560 points wide; it was looked at at 280, 340 and 520 offscreen.
- Neutral colour only: the bars and the interval are the label colour dimmed, the two badges use
  the app's status tones (agreement, small sample), and nothing is blue.

## Questions

| Outcome | A day counts as a hit when | Needs |
| --- | --- | --- |
| `gap_fill` | a later bar's range touches the previous close; days with a gap only | a previous day |
| `day_up` | it closed above its open (a flat day is not up) | |
| `prior_high_break` | its high went above the previous day's high | a previous day |
| `prior_low_break` | its low went below the previous day's low | a previous day |
| `first_bar_high_break` | a later bar traded above the first bar's high | 1H bars, 2+ bars in the day |
| `first_bar_low_break` | a later bar traded below the first bar's low | 1H bars, 2+ bars in the day |

Conditions narrow the days: weekdays (Mon–Sun), the gap's side and its size in percent of the
previous close (the size ignores the sign), and whether the previous day closed up or down. A
condition about the gap or the previous day leaves out a day that has no previous day.

## What a day is

- **A US stock** (source Alpaca): the regular session in New York. An hourly bar belongs to a day
  when it overlaps 09:30–16:00 on a weekday (`pine::trading_session::MarketSessions`, with
  `pine::timezone` for daylight saving). A daily bar is its UTC date, which is the date of its
  04:00/05:00 UTC stamp.
- **Everything else**: the UTC date.
- A previous day more than five calendar days earlier is a hole in the data, so nothing is compared
  with it.
- Bars are the market's own candles up to the replay position, never a constructed chart type's
  bars (a Renko brick is not a trading day), the same as `get_candles`.

## Limits and inferred choices

- **This build has no bars under an hour.** Edge-stats works on minute bars. So here the opening
  range is the first *hourly* bar, not 30 minutes, the time of a fill is only known to the bar
  (reported as a bar index, with the hours it covers), and weekly or longer timeframes are
  refused (`Unavailable::TooCoarse`, `UNSUPPORTED` over MCP). The first-bar questions need 1H.
- **A stock's first hourly bar may start before 09:30** (Alpaca's hourly bars are on the clock
  hour and include extended hours), so a day's open may be a pre-market price. The result carries
  `Note::BoundaryOpen`; the daily timeframe has no such blur.
- **A market that trades round the clock has almost no gaps**, so gap questions on crypto are
  mostly empty; the result says why (`Note::NoGaps`).
- **"The halves agree"** means the older and newer half's Wilson intervals overlap. Edge-stats
  prints the same words without saying by what rule, so this is inferred. Halves are shown from 10
  days each.
- Percentiles of the time of a hit are nearest-rank.
- Holidays are not modelled; a missing day simply is not there.

## Verified and not

Verified by unit tests: the interval against the published example, each outcome and filter on
hand-made bars, hourly folding, New York hours in winter and summer, the cache (the answer is
worked once per change of bars or question, never per frame), the panel laying out in every state,
the menu entry and the panel following its chart, and the MCP tool end to end over a loopback
socket with its permission.

Not verified: a real window (the panel was only rendered offscreen, in both themes, at three
widths and in the empty, small-sample, hourly and unavailable states), real Alpaca
hourly bars (how the first bar of a real day starts), and numbers against edge-stats on the same
data.
