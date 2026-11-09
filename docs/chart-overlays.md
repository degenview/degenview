# Chart overlays (Compare)

Markets drawn over a chart, on the chart's own canvas and time axis. TradingView calls it Compare;
this is the same idea. A chart can compare any number of markets: the model is a vector, nothing
caps it, and what is bounded is the work (see [Resource use](#resource-use)).

Overlays are not charts (a chart card is still added next to another with Add Chart or ⇧↵ in the
picker), not indicators, not Pine plots and not drawings. They are owned by one chart, have a stable
id, and nothing a script, alert, drawing or paper order reads looks at them: those keep reading
`Chart::symbol`, `source` and `candles`, and tests pin that.

## Where things are

| Piece | File |
| --- | --- |
| Model: `ChartOverlay`, `OverlayList`, `OverlayCommand`, scale choice, ordering, duplicate policy | `src/model/overlay.rs` |
| Candles onto the chart's bars (midpoint rule) | `src/model/overlay_align.rs` |
| Rebase onto the main series, own-scale fit, legend value | `src/model/overlay_scale.rs` |
| What the card is told about a market's data (`OverlayFeed`, `OverlayStatus`) | `src/model/overlay_feed.rs` |
| Data: one fetch per market, shared, scheduled | `src/workspace/overlays.rs` (`OverlayHub`) |
| Add, edit, remove as undo steps; Regular → Percent on the first add | `src/workspace/overlay_actions.rs` |
| Resolve against the bars in view, lines, tags, crosshair rows | `src/ui/chart_card/overlays.rs` |
| Legend chips and menu | `src/ui/chart_indicator_strip.rs` (`Kind::Overlay`) |
| Settings page | `src/ui/chart_settings.rs` (`Tab::Overlays`, "Compare") |
| Object tree section "Compared symbols" | `src/model/object_tree.rs` (`Key::Overlay`), `src/workspace/context.rs` (`apply_tree_action`) |
| Picker mode | `src/ui/symbol_picker/` (`PickMode`, `Output::Overlay`) |

## Behaviour

**Adding.** There is no toolbar button. In the symbol picker (⌘K) ⌥Return draws the highlighted
market over the focused chart once and closes, the highlighted row has a Compare button next to its
plus, and the row's right-click menu has "Compare with …". The chart settings' Compare page has "Add
symbol…", which opens the picker in compare mode (`PickMode::Compare`): Return draws the highlighted
market and the picker stays open, so several can be added in a row (Esc closes). ⌘K, Return (replace)
and ⇧Return (add a chart) are unchanged.

Nothing is added for the chart's own market or a market already compared (compared by
`InstrumentId::key()`, so `BTC` and `btcusdt` are one). A chart that is a line-only source
(prediction markets, CoinMarketCap) or draws a price-based type (Renko, Kagi, Line Break, Point &
Figure, Range) takes none: those types have no shared time axis. Overlays already on a chart that is
switched to such a type stay in its configuration, are not drawn, and the legend says why.

**Time alignment.** By the *midpoint of the candle's span*, never by index and not by open time. Each
chart bar owns the span from its open to the earlier of the next bar's open and one nominal bar later;
an overlay candle lands on the bar that holds its midpoint. This keeps Alpaca's New York-midnight
daily bar and a crypto 00:00 UTC day on the same date, and keeps a weekday-only market from taking a
neighbour's weekend. A bar with no candle has no value: the line breaks, nothing is carried forward,
the crosshair reads "—". Only the bars in view are aligned, so the cost follows the screen.

**Scales.** Per overlay:

* *Chart's scale* (default, TradingView's "Same % scale"): the overlay is rebased so its first value in
  view is drawn at the main series' reference price (the close of the first bar in view), scaled by the
  ratio. On the Percent and Indexed to 100 axes that reads as the overlay's own change from its first
  visible value, which moves with a pan, as TradingView's does. Because a ratio is multiplicative the
  logarithmic axis works unchanged, and a coin at 0.2 and one at 100,000 compare. Adding the first such
  overlay to a chart on the Regular axis switches it to Percent (TradingView does the same); removing
  overlays never switches it back. On Regular or Logarithmic the legend and tags read real prices.
* *Its own scale* (TradingView's "New price scale" without the axis column): the group's visible values
  fill the plot height with an 8% margin. There is no second axis; the value is in the tag at the
  line's end and in the legend. Overlays that carry the same group id share one fit ("Scale of X").

An overlay with no usable value in view, or a chart with no reference price, is not drawn (legend:
"No data in view").

**Drawing.** Under the main series, over the grid and the Pine visuals behind the chart, clipped to the
plot, one line per run of bars (lines break at gaps); a single isolated bar is a dot. Order is the list's
order. Last value: a tag on the axis for a chart-scale line (it reads as the axis does), a pill in the
plot's right edge for an own-scale line; neighbours are pushed apart. The price range grows to fit
rebased lines. The crosshair adds a ring on each line and a box under the OHLC box with one row per
overlay on the chart (colour, name, value, a dash where it has no candle or nothing in view; hidden and
blocked ones are not listed); rows that do not fit are counted ("+N more") and stay in the legend and
settings. Lines are painted by `paint_frame`, the frame snapshots share, so snapshots include them
(`snapshot_tests` compares the two shape for shape).

**Replay.** The card is handed bars already cut at the replay cursor, and an overlay candle can only
land on one of them, so the lines, baseline, price range, tags, crosshair and snapshots never see a
later candle. Paging older history follows the chart's own oldest candle, never a replay position.

## Legend and settings

Compared markets are chips in the indicator strip after the indicators and scripts: colour dot, name,
eye / settings on hover, an x, and a right-click menu (Settings, Hide/Show, Scale ▸ Chart's / Its own /
Scale of …, Visual order ▸ Bring to front / forward / Send backward / to back, Remove). More than three
fold into the strip's "+N" list, which scrolls and shows each market's newest price. The Compare page
of the chart settings (`ui/chart_settings_compare.rs`) has one card per market. The header always shows
the line as drawn (colour, width, dash), the name, the exchange and how the data is doing (last price,
"Loading…", "Waiting to load", or an orange "Couldn't load: …" / "Same market as the chart" / "Not
shown on price-based chart types"), with the eye, a menu (Bring to front / forward, Send backward / to
back, Remove) and a chevron; clicking the header opens the settings: colour (the palette, or any), line
width and style, the price of each candle the line follows (open, high, low, close, HL2, HLC3, OHLC4),
scale with a sentence on what it does, and the last-value tag. Cards start open while there are three
or fewer and closed after that, and what the user opened or closed is kept while the sheet is open. Past
six markets a search field appears (name, symbol, exchange); reordering is off while it filters. The
row above the cards has "Add symbol…", a count ("12 markets · 1 hidden") and a menu: Show all, Hide
all, and Remove all…, which asks first. While any visible market is on the chart's scale and the axis
is Regular or Logarithmic, a notice says the axis cannot compare them and offers Percent and Indexed to
100. With nothing compared, the page invites the first one. Every edit is an `OverlayCommand` and one undo step (`Snapshot::Overlays`); an edit that changes
nothing leaves none. The add that switches Regular to Percent carries the old mode in its step, so undo
puts the axis back and redo switches it again; no other step touches the mode. A colour dragged in the
picker reports every frame and is one step (`last_overlay_style`, the 800 ms window drawings use).

The Object tree lists the compared markets too, in a "Compared symbols" section under the symbol, in the
list's order: an eye (hide or show the line), a trash (remove), and a double-click or Settings… that
opens the Compare page. No lock and no reordering there (use the page or the legend). The tree runs the
same `OverlayCommand`s through `apply_overlay_command`, so each is one undo step.

## Data

`OverlayHub` holds one entry per market (`InstrumentId::key()`), shared by every overlay on every chart
of the workspace that names it, and dropped when the last holder goes. It composes the services the
chart uses (`service::fetch_klines`, `fetch_klines_range`, `work_queue::submit_for`,
`merge_fetched_candles`, `OlderHistory`) and adds no service of its own.

* **Per frame**: `sync_chart` tells the hub what a chart's overlays name (a comparison, no allocation,
  when nothing changed); `poll` drains answers and starts what is due; the workspace hands each card
  the feeds only when the hub's version, the overlays or the chart's market changed.
* **Remove or close while loading**: dropping the entry bumps its generation, so a job not yet started
  does nothing and one running answers a receiver that is gone.
* **Timeframe change**: every entry is reset and refetched; an in-flight answer for the old timeframe
  is discarded.
* **Older history**: a market pages back (`PAGE` candles at a time, on the usual gate) while it starts
  later than the oldest candle any chart holding it has, and stops at the provider's start or the
  `MAX_CANDLES` cap. It never pages for a hidden overlay.
* **Live**: Binance overlay symbols join the one live socket (deduplicated with the charts') and a
  candle updates every holder. Other providers refresh over REST.
* **Errors**: a failing market shows its own error, is retried after 15 s (45 s for CoinGecko and
  DexScreener) and never touches the others or the chart. A missing Alpaca key is that error.

## Resource use

There is no cap on how many markets a chart compares. Work is bounded instead:

* at most `MAX_IN_FLIGHT` (4) overlay requests run at once, the rest are *queued* and start as slots
  free, first loads before older pages before refreshes;
* a market refreshes every `max(5 s, 0.25 s × active markets)` (15 s floor for the rate-limited
  providers), so the request rate stays near four a second whatever the count;
* hidden overlays are not fetched and do not join the live socket;
* one market on several charts or overlays is one fetch, one candle vector (an `Arc`), one entry;
* the card aligns only the bars in view and keeps the result per overlay until the candles, the
  window or the bars change.

Memory is the candles held: `markets × candles × 56 bytes` (for example 100 markets × 2,000 candles is
about 11 MB). Candles are paged back only as far as the chart's own oldest candle, which `MAX_CANDLES`
(100,000) caps. A layout with overlays stores about 200 bytes per overlay in its record; iCloud Sync's
record limit (about 900 KB) is reached at some thousands, and a layout that large is refused by the
sync validator, not by the app. The cost per frame as the count grows is measured by the ignored test `overlay_cost` in
`ui/chart_card/overlay_tests.rs` (`cargo test --release overlay_cost -- --ignored --nocapture`); its numbers
are not recorded here because it has not been run yet.

## Persistence

`PersistedChart.overlays` (and `ChartConfig.overlays`, so an edit dirties the layout and autosaves) hold
the configuration only: instrument, title, visible, colour as `#RRGGBB`, width, dash, source, scale,
tag. It is left out of the row while empty, so a layout that uses none is byte-for-byte what an older
build writes. A market that cannot be fetched (delisted, no Alpaca key) stays in the configuration and
shows its error. Overlays are not part of a chart template and not of a template's "apply to all".
`Chart::replace_market` keeps them; one that becomes the chart's own market is kept, not drawn, and
says so. iCloud Sync needs nothing new (no table, no setting key); an older build that re-saves a
layout drops the overlays, which makes the usual "conflict" copy, as `scale_mode` did
([iCloud Sync](icloud-sync.md#newer-versions)).

## Verified, and not

Verified against TradingView's support article and charting-library documentation: the three
placements (same % scale, new price scale, new pane); that the percent axis starts every series at 0% on
the first visible bar and so changes with a pan; that Indexed to 100 sets the first value to 100.

DegenView's own decisions, not verified against TradingView: lines under the main series; the picker
staying open after a compare pick; ⌥↵; the baseline when an overlay starts after the first visible
bar (its own first visible value); the colour palette; hide and order controls; the 8% margin of an
own scale; switching Regular to Percent only for the first line on the chart's scale.

Not done, and different from TradingView: no "new pane" placement; no second price axis column (an
own scale has a tag and a legend value, no ticks); overlays are lines only (no candles, bars or areas
for a compared market) with solid, dashed or dotted strokes; no drag-and-drop reordering (menu and
buttons); no MCP tool (a snapshot taken over MCP includes the lines, `list_charts` does not name them);
no entry in the chart's right-click menu; a Regular axis shows rebased prices, so the first overlay
switches it. The axis column is as wide as the widest last-value tag of a line on the chart's scale. Checked headlessly (tests, snapshot parity) and by looking at a macOS release build with five markets
on one chart; the compare picker, the settings page and the light theme were not looked at in a window,
and neither were Linux and Windows. Stocks need an Alpaca key and were not tried. No performance figure
has been measured.
