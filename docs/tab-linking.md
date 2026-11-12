# Tab Linking

Tabs tagged with the same colour keep chosen parts of their chart in step, in any window. This is
not [Link charts](chart-sync.md), which carries a pan or zoom between the charts of one layout;
Tab Linking is between tabs and only ever touches the chart the user is working in.

Code: `model::tab::link` (pure model, hub and tests), `host::tab_link_host` (the switches, saved),
`workspace/tab_link.rs` (a tab's side), `DegenViewApp::tick_tab_links` in `app.rs` (the tick),
`ui/window/tab_link_menu/mod.rs` (the menu section), and the underline and the right-click menu in `ui/window/tab_bar.rs`.

## What TradingView says, and what is ours

Read from TradingView's pages (the Desktop posts of Nov 2025 and May 2026, "Symbol syncing between
tabs", "How to sync the charts of my layout"):

- Tabs share a **colour tag**; tabs with the same colour form a linked group. Stated.
- Symbol and Interval are chosen "within each" group, and the options can be used together or
  separately. Stated. Interval syncs across tabs **and windows**. Stated.
- Time and Date range exist as sync options for chart tabs. Stated, **but neither is defined**.

Everything else is **not stated by TradingView, so it is a decision here**:

| Question | Decision |
| --- | --- |
| Where are the switches kept? | Per colour group: one set per colour, shared by its tabs (the user's choice). |
| Defaults | Symbol on, the rest off, as the screenshot of TradingView's menu shows. |
| Where are the options? | At the bottom of a chart tab's right-click menu, after a separator (the user's request); not in the toolbar. They act on the tab that was clicked, showing or not. |
| Does clicking the chosen colour unlink? | Yes (its tooltip says so). |
| Where does the tab show its colour? | A line under the tab in its colour, showing or not; a tab with no colour has no line (the old blue selection line is gone). Its tooltip also says "Linked tab: Orange". |
| Who wins when a tab joins? | The joiner adopts the group's latest values for the enabled switches; the tabs already in it are never touched (the user's choice). |
| What is Time, what is Date range? | Time is the **position** (the time at the right edge of the chart), Date range is the **zoom** (the span of time shown). These are `ChartSyncSettings::horizontal_position` / `horizontal_zoom`, so Link charts' table applies: both on gives the same time range. |
| Restart | The colour is saved with the tab, the switches in a setting. No event is saved, and nothing moves at startup. |
| Replay | A tab in replay takes neither a symbol nor an interval (it would move the replay's timeline from under it); it catches up with the group's next change. |
| Multi-chart layouts | The **focused market chart** leads and follows. Its siblings are not moved by the link; they follow only through their own Link charts when the user navigates. |

## Rules

- A tab is in one group or none. The hub reads membership from the tabs every frame, so a closed,
  moved or recoloured tab cannot leave a stale subscriber, and a group with no tabs keeps nothing.
- Only a user's action is announced: the symbol picker, a watchlist click, the timeframe buttons,
  `degenview_set_timeframe`, and (with Time or Date range) the focused chart's own pan or zoom.
  Applying a delivery goes through the same commands (`switch_chart`, `reload_for_timeframe`) but
  never touches the outbox, so nothing echoes. There is no value comparison in the loop guard.
- A tab's outbox has one slot per kind, newest wins (so BTC → ETH → SOL → AVAX is one delivery,
  AVAX), and a delivery with an older sequence than one already applied is dropped.
- Not announced: restoring a session, loading a saved layout, adding a chart, a chart that is not
  focused, an unlinked tab (its outbox is cleared the moment it is unlinked).
- The symbol is the whole watchlist row (provider, symbol, title, artwork), never a bare ticker.
  The chart keeps its type, settings, indicators, Pine instances, drawings and timeframe.
- When several tabs speak in one tick, every tab of the group ends on the newest value: a tab is
  never told something older than what it said itself.
- Choosing a market that is already open on the tab (from the watchlist) is announced like any
  other change of the focused market.
- A symbol that cannot load is put back: the chart's first fetch fails with nothing on it, so the
  tab shows what it had and says so in the chart notice. The rest of the group is unaffected.
- A tab with no market chart (a blank tab, a portfolio card in focus) has nothing to change and gets
  no chart invented for it.
- Tab Linking never touches the crosshair, drawings, alerts or Pine scripts.

## Time and Date range

A pan or zoom of a linked tab's **focused chart** becomes a `HorizontalChange` (a time range, never
pixels or bar indices) in the tab's outbox, newest wins. The other tabs get it as an event in a
`SyncHub` of their own (origin `u64::MAX`, no real chart has it) and their focused chart follows it
with `follow_sync` before it draws, exactly as a chart follows another chart of its layout:

| Time | Date range | A pan | A zoom |
| --- | --- | --- | --- |
| on | off | their right edge goes to the leader's time; their zoom is kept | – |
| off | on | – | their bar count scales by the leader's factor; their position is kept |
| on | on | as above | they show the leader's time range |

A chart with no bars yet is not asked to follow, so the event waits for its candles, and so does
a tab that is not showing (it follows when it is next drawn). Nothing jumps when a switch is turned
on; the next navigation takes effect, and an event that arrives under other switches than the last
one starts the tab's hub afresh. A symbol or interval change (the group's or the user's own) resets every linked
chart to its newest bars, the way it does for the user, and drops a view the tab was still waiting
to follow, so the old series' view is never replayed on the new one. A zoom waits for a chart that
has no bars yet: a frame without bars does not count towards ending the zoom.
Price-based chart types (Renko, Kagi, Line Break, Point & Figure, Range) neither lead nor follow.
Vertical scales are never carried.

## Persistence and iCloud

`PersistedWorkspace.link` (`"orange"`, ...), skipped when none, so old rows decode and a row from a
newer build with a colour this one does not know reads as not linked rather than failing the whole
`tab` table. The switches are the `tab.link_toggles` setting. The `tab` table and that key are
both local to the Mac (`sync::policy`), so nothing here reaches CloudKit, and the colour is not in
a saved layout.

## UI

The section is called **Link Tabs** and is drawn after a mock-up the user supplied: the title and
"Sync this tab's charts with other tabs.", a **Link Color** card with the nine swatches (the chosen
one ringed and ticked, so it is not told by hue alone) and, **only while a colour is chosen**,
**Sync what** with Symbol, Interval, Time and Date range as a grid of cards (glyph, name, switch; the
whole card is the control, its tooltip says what it does) and a picture of two linked tabs with a
line of explanation. Choosing the chosen colour again unlinks and the lower part goes away. There
is no icon beside the title and no Unlink Tab / Done buttons (the mock-up has them; they were
dropped on request): the menu closes like any context menu.

Where it departs from the mock-up on purpose: nothing is blue (a card that is on has a lighter
fill and a neutral outline, the switches are the app's neutral ones and Done is the neutral primary
button), and the glyphs are the app's own set. It is 332 pt wide to fit the context menu.

## Verified, and not

- The model, hub, persistence, the workspace's apply and probation, the viewport path through real
  pointer events (`ui/chart_card/tab_link_tests.rs`: two tabs, the four combinations of the
  switches, different intervals, a chart that has no bars yet, no echo) and the menu section's
  clicks (`the_tab_menu_ends_with_the_tab_linking_section`) are covered by tests. `render_grid`'s
  own wiring of the same calls (`follow_sync` on the focused chart, `link_change` into the
  outbox) is covered only by review and by driving the app. The menu was drawn headlessly in both
  themes (`gallery_of_the_tab_menu`, `gallery_of_the_tab_linking`) and looked at.
- **Not run in the real app** in a first pass of this document; see the end of the PR for what was
  driven by hand. Not verified at all: several physical monitors, Retina and non-Retina side by
  side, a real offline run, Windows and Linux.
- The apply latency is the root frame plus a repaint of the target window (one or two frames).
  A tab in a background window applies a symbol or interval at once but draws it when shown.
