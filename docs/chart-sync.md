# Linked charts

How panning and zooming one chart of a workspace carries over to the others. The brief this
implements is the "Linked Chart Zoom and Horizontal Position" document; this file records what was
built, what is verified, and what is not.

Code: `model::chart_sync` (pure model and tests), `ui/chart_card/sync.rs` (a chart's side),
`ui/link_charts.rs` (the popover), the tool-strip tile in `ui/tool_sidebar.rs`, and the loop in
`workspace::ChartWorkspace::render_grid`. Tests: `model/chart_sync/tests.rs` and
`ui/chart_card/sync_tests.rs` (several charts through egui's real pointer pipeline).

## What TradingView does, and what is ours

Verified from TradingView's public pages (support article and May 2022 announcement):

- *Date range* is one option of *Sync in Layout*, next to symbol, interval, crosshair and time.
- Scrolling or changing the visible date range of one chart carries it to the other charts in the
  same layout window.
- It works only on time-based chart types (bars, candles, line, area, baseline, Heikin Ashi).

Not stated by TradingView, so **inferred**: bar spacing and widths (the pages say nothing on
whether bar spacing is synced; a release-notes line claiming "matching bar spacing" could not be
found, so nothing here relies on it), behaviour with different intervals, less history, whitespace,
replay, how a toggle takes effect, and whether position and zoom can be split.

DegenView-specific:

- Position and zoom are **two** switches, not one "date range" switch (the brief's request).
- Different timeframes and different widths are first-class (see below).
- Charts with price-based types opt out, and the popover says which.

## The model

`HorizontalViewport { visible, right_offset }` stays the only horizontal state. It describes the
bars between two slot edges, `right = len - right_offset` and `left = right - visible`, with the
Canvas right margin as free space past `right`. `TimeAxis` maps a chart's slot edges to timestamps
and back, so a viewport *is* a time range.

- **Position** is the time at the right edge. At the newest bar it is the end of that bar, so
  charts of any interval agree on "now" and the margin (a presentation setting) never enters.
- **Zoom** is how many bars span the range. A zoom keeps the right edge (the app's existing
  anchor), so it never moves the position.

`TimeAxis` interpolates between a bar's open and the next open: a weekend inside a stock chart is
one slot wide, so the map is continuous and strictly increasing and a chart never oscillates
between Friday and Monday. Beyond either end it continues at the length of the first or last bar:
empty space is a position in time, never an invented bar.

| Position | Zoom | A pan | A zoom |
| --- | --- | --- | --- |
| off | off | – | – |
| on | off | followers' right edge goes to the leader's time; their zoom is kept | – |
| off | on | – | followers' bar count scales by the leader's factor about their own right edge; their position is kept |
| on | on | as above | followers show the leader's time range (count from the time span, right edge from the time) |

- Charts are matched by **time**, never by bar index. A 24-hour span is 24 hourly bars on one chart
  and 96 quarter-hours on another; a range over a weekend is matched in time, not in bars.
- Different **widths** need no special case: the viewport has no pixels, so linked charts show the
  same bars across their own plots (a narrower chart shows them thinner). The user's own zoom is in
  bars, as before.
- Zoom-only mode scales from the follower's count **when the burst began**, not step by step, so
  rounding cannot accumulate over a long wheel gesture. A burst ends with the first frame that does
  not zoom.
- A chart at its newest bar sends followers to **their** newest bar, so "go to latest" works even
  when one feed lags by a candle.

## Who leads, and how loops are prevented

`ChartWorkspace` owns the settings and a `SyncHub`; nothing is global, so other workspaces, tabs
and windows are never touched (the boundary is the layout, not the native tab group).

1. Before a chart draws, `ChartCard::follow_sync` asks the hub for anything new and edits the
   chart's own viewport, so even its header's visible-change figure reads the new window.
2. The chart draws. `body` remembers the viewport after live bars were accounted for and, once
   the frame's input is processed, diffs it: drag, wheel, swipe, axis drag, axis double-click, zoom
   and reset buttons and Escape all edit that one viewport, so they all produce the same
   `HorizontalChange`.
3. `publish_sync` hands the change to the hub with the chart's id and a growing sequence number.

The chart under the user's hand is the leader for that frame; any chart may lead the next. A
follower's viewport change is made *before* the frame's baseline is taken, so it is never a change
of its own, and the hub never delivers an event to its origin or twice to one chart. There is no
float comparison anywhere in the loop guard. A chart being dragged is never moved by the hub.

Only the newest event is kept: a position is a time and a zoom is measured from its burst's start,
so skipping one loses nothing. Charts drawn **after** the leader in the grid (left to right, top to
bottom within a column) follow in the same frame; one drawn before it follows on the next frame,
which the workspace requests immediately. This is the one place the "same frame" goal is not met in
a single pass: egui's `request_discard` would repaint the grid twice every frame of a drag and
flashes a debug warning, so it was not used. See the measurements below.

Live data is never navigation: a bar that arrives changes the offset before the baseline, so it
publishes nothing. Replay cuts the bars a chart receives; the time map reads those, so
unrevealed bars cannot be shown and nothing here steps replay.

## Vertical independence

`HorizontalChange` carries no price information and `follow` reads and writes only
`HorizontalViewport`, so a price range, zoom, autoscale or log scale cannot cross charts. A diagonal
drag publishes only its horizontal part; a price-axis drag changes only the vertical state and
publishes nothing.

## Less history, gaps, whitespace

- A follower whose data starts after the leader's time stays at its oldest bars (with a screenful
  in view, the viewport's rule) and remembers it has not arrived. When older candles load it moves
  on, without any new gesture. The leader is never pushed back.
- Space past the newest bar is followed in time, within the limit every chart has.
- A chart with no bars is left alone.

## Which charts take part

A chart takes part when it is time-based: every standard type, Heikin Ashi, lines and areas, and the
line charts of prediction markets. **Renko, Kagi, Line Break, Point & Figure and Range** neither
lead nor follow: their slots are price moves, not elapsed time, and a time borrowed for them would
make the charts disagree about the date. The popover lists such charts ("Not linked: …"). Portfolio
cards have no time axis and are ignored (the control is disabled with fewer than two market charts).

## Settings and persistence

`ChartSyncSettings { horizontal_position, horizontal_zoom }`, both **on** by default, lives in
`PersistedWorkspace.sync`, `SavedView.sync` and `LayoutSnapshot.sync` (all `#[serde(default)]`, so
rows saved before linking open linked and nothing is migrated; a saved choice, including
`ChartSyncSettings::UNLINKED`, is kept). The brief asked for both off so that existing layouts keep
behaving as before; the default was changed on request, so a layout with several charts now moves
together from the first pan or zoom. Linking is part of the layout fingerprint, so unlinking a
named layout with Autosave saves it and an unnamed one shows "(unsaved)". The session saves it with
the tab. The settings are kept while the layout holds one chart.

Turning a switch on moves nothing: it takes effect on the next navigation. Turning one off leaves
every chart where it is. Changing a switch resets the hub, so a stale event is never replayed.

## UI

A tile on the left tool strip, under the hide-all-drawings eye behind its own separator: a chain on a
grey wash (never the blue accent) while linked, a broken chain while not, and not drawn at all while
the layout holds a single chart (or on a window too short for the options). It opens a popover to
the right of the strip with two switches, each a full-width row with its description. Rows take the keyboard (Tab, Space or
Enter) and show a focus ring, carry checkbox accessibility info, and use the app's theme colors.

## Measurements

`cargo test --release --lib linked_drag_frame_cost -- --ignored --nocapture` drags the first chart
of N charts headlessly (egui's pipeline, no GPU), unlinked against both switches on:

| Charts | Candles each | Unlinked | Linked | Events for 60 frames |
| --- | --- | --- | --- | --- |
| 2 | 10,000 | 64 µs | 66 µs | 58 |
| 4 | 10,000 | 108 µs | 118 µs | 58 |
| 4 | 100,000 | 430 µs | 488 µs | 58 |

One event per drag frame (the first frame of a press moves nothing), none without input. Linking
adds about 10 % on top of a frame that is already drawing every chart; the cost is the follower's
`TimeAxis` lookup (a binary search). There is no allocation per event: a `HorizontalChange` is a
few `f64`s, and the hub allocates only when a zoom burst begins. Not measured: allocation rate
and CPU in the running app, hundreds of drawings or several Pine overlays together with linking.

## Not verified, not done

- **Not run in the real app.** Nothing here was driven with a physical mouse, and the strip
  tile and popover were not looked at on screen (only drawn headlessly by a test): the
  behaviours above were exercised through egui's input pipeline. Check the icon, the popover's
  spacing in light and dark, and the keyboard focus ring by eye.
- Trackpad pinch does not zoom a chart in this app (only the wheel and a vertical swipe do), so
  there is nothing to link; it would work once added, as it would edit the same viewport.
- No keyboard shortcut navigates a chart today; a future one would link for the same reason.
- Position-only linking of charts with different timeframes aligns the **right edge**. With
  different zoom the left edges differ by design.
- Combined mode matches the time range at a **zoom**; a later pan moves the right edge and keeps
  each chart's bar count, so across gaps (weekends) the left edges can drift a few bars until the
  next zoom.
- TradingView aligns charts when the option is switched on; here the first navigation does. This
  was chosen over jumping the charts under the user (the brief prefers it) and is unverified.
- A chart drawn before the leader trails it by one frame (see above).
- Replay and the paper-trading, alert and Pine overlays were not exercised with linking beyond
  sharing the same viewport transform.
