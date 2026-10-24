# Chart navigation

How the chart is panned, zoomed and scaled. The specification this implements is
`chart-dragging.md`; this file records what was built, what is verified, and what is not.

## One viewport

`model::viewport::ChartViewport` is the only description of what part of a chart is on screen. It
lives in `ChartCard::viewport`; every gesture edits it and every painter reads what it resolves to.
Nothing else keeps an offset, a bar count or a price range.

| State | Meaning |
| --- | --- |
| `horizontal.visible` | Bars across the plot (zoom), 20 to the larger of the bars held and the default 120, so a short series (young pool, coarse timeframe) keeps a deep market's bar width and leaves blank space on the left rather than jumping to fit. Saved as `candle_count`. |
| `horizontal.right_offset` | Bars scrolled back from the newest, as an `f64`. Negative is empty space past the newest bar. `0` is *follow latest*. |
| `vertical.zoom` | The price-axis drag's stretch of the *automatic* range about its centre. Saved as `y_zoom`. |
| `vertical.manual` | A price range the user placed by dragging the chart. While set, autoscale is off. |

`canvas::visible_window(len, visible, right_offset, reserved)` turns it into the bars to draw. The
offset is continuous, so the first and last bar may be cut by the plot's edge; `Window::lead` is
the (fractional) number of slots between the plot's left edge and the first shown bar. Painters
that draw bars are clipped to the plot for that reason.

The offset counts from the **newest** bar. Two consequences are deliberate:

- Older candles loading in front never move the view.
- A bar that arrives while the previous newest bar is on screen (`HorizontalViewport::shows_newest`:
  its centre is inside the plot, so the offset is below the reserved margin plus half a bar) leaves
  the offset alone: the new bar takes the newest bar's place and the chart slides left one slot,
  whether the chart is at offset `0`, scrolled back a few bars or dragged into empty space. This is
  Lightweight Charts' `shiftVisibleRangeOnNewBar` rule (`ChartModel.updateTimeScale`), the open-source
  engine of TradingView's charts.
- A bar that arrives while the newest bar is already out of view (scrolled back past the margin) is
  added to the offset (`ChartCard::keep_view_on_its_bars`) so the same bars stay on screen.
- A bar that arrives mid-drag with the newest bar on screen moves the chart a slot under the
  pointer, as there; the compensation for a held view is applied to the drag too.

The Canvas right margin stays the default placement of the newest bar; panning into empty space is
temporary viewport state and never writes the Canvas setting.

## Gestures

Which gesture a press starts is decided by where it lands (`navigation::Regions`) and, over the
plot, by whether a drawing gesture claimed it (`DrawingLayer::claims_drag`, checked first).

| Input | Result |
| --- | --- |
| Drag the plot | Pan. Follows the pointer exactly (`PanGesture`: measured from the press, no rounding, no dead zone, no jump on release). |
| … vertical part | Once the drag strays 4 pt vertically the price range goes with it and autoscale turns off; the distance already travelled counts, so the chart never lags. A drag that stays horizontal leaves autoscale on. |
| Drag a script pane | Pans through time only: panes share the time window. |
| Drag the price axis | Stretches the range. Autoscale stays on and `zoom` changes; on a manual range it scales that range about its centre. Never pans sideways. |
| Drag the time axis | Stretches the bars (`visible`); the newest bar stays put. Dragging right spreads them. |
| Double-click the price axis | Autoscale at the natural scale. |
| Double-click the time axis | Newest bars at the default spacing. |
| Reset button | Both of the above. |
| Scroll / zoom buttons | As before; the scroll buttons stop at the newest bar, only a drag goes on into empty space. |
| Wheel | Zoom (unchanged). |
| Sideways trackpad swipe | Scrolls through time (swiping right shows older bars). Mostly vertical swipes still zoom. |
| Escape during a drag | The view returns to where the press found it; the rest of the drag is ignored. The Escape is reported as consumed so nothing else answers it. |
| Cursor | Crosshair over the plot (unless a drawing chose one), a grabbing hand while panning, resize arrows on the axes. |

A drag that began on a drawing handle or body edits the drawing; a drag on a **locked** drawing pans
(the layer starts no gesture for it); with a drawing tool active the drag draws. Panning produces no
`DrawingCommand`, so it is never an undo step, and drawings hold time+price anchors that are never
rewritten.

A drag ends with the button: if egui reports the button up (released outside the chart, window lost
focus) the gesture is dropped, so there is no stuck drag.

Linking the charts of a workspace is built on this viewport: see [linked charts](chart-sync.md).

## Price range

`VerticalViewport::resolve(auto, logarithmic)` is the single place the displayed range is chosen:
the manual range if it is valid for the scale, else the automatic range stretched by `zoom`. A pan
moves the range in the scale's own space (log space on a log scale), so a log drag changes prices by
ratio and can never produce a non-positive minimum; a drag too far to draw is refused and the
previous range stays. A manual range does not follow the data.

Percentage and indexed scales exist as [price scale modes](price-scale.md). They are linear in the
price, so they change nothing here (the range, pan and zoom are the same), and a mode switch keeps a
manual range that is valid for the new map. `logarithmic` in these calls is the frame's effective flag:
false while the view holds a price a logarithm cannot take. Not present in the app, so not
implemented: inverted scales. "Scale price chart only" does not exist either; autoscale semantics for
indicators are unchanged.

## Persistence

`PersistedChart::viewport` (`SavedViewport`, `#[serde(default)]`) keeps `right_offset` and the
manual range next to the existing `candle_count` and `y_zoom`. It is chart/layout state, not
appearance: templates (`ChartAppearance`) neither carry nor reset it, and `LayoutSnapshot` still
ignores it, so panning does not mark a layout dirty. Tab switches and window resizes keep it (it is
in bars, not pixels); a symbol or timeframe change resets it as before (`reset_view`).

## Older history

`model::history` holds the pure rules; `workspace::Chart::{request_older, receive_older}` own the
request. When the view is within two screens of the oldest candle held (`ChartCard::near_oldest_bar`),
or right after the first fetch (`OlderHistory::untouched`, so the first page is already there when
the user scrolls), the chart asks `service::fetch_klines_range` for the 1,000 candles before it,
through the work queue. One request is out at a time, a failure waits 15 s, an empty page marks the
start of history, and a page for a market or timeframe the chart has left is dropped. A refresh
keeps the older candles it did not bring (`history::with_older`) once a page has been loaded; the
chart holds at most 100,000 candles.

`DataSource::supports_older_history(timeframe)` says what can page. Coarser bars (1w, 1M, 3M, 1Y)
are folded from finer pages with `service::fold_calendar`, whose first bucket is dropped when the
page was cut by the request, so a bar is never split across two pages.

| Provider | Older pages |
| --- | --- |
| Binance | Yes, every timeframe (3M / 1Y fold monthly klines). |
| Coinbase | Yes, every timeframe (daily or hourly pages of 290, folded). Not verified live. |
| Alpaca | Yes, every timeframe (needs the API keys; `start`/`end` bars). Not verified live. |
| DexScreener | Hourly only. GeckoTerminal's free API serves the last 180 days (`before_timestamp` older than that is HTTP 401, taken as the start of history), so daily bars arrive whole (about 180) and 1W / 1M / 3M / 1Y are shallow by nature. |
| CoinGecko, prediction markets, CoinMarketCap | No: no time-range fetch is implemented. The chart stops at what a refresh brings. |

There is no loading indicator; the view does not move while a page loads.

## Scrolling to a bar

`ChartCard::scroll_to_time(time_ms, candles)` puts the bar holding a time in the middle of the view
(`right_offset = bars after it - visible / 2`, clamped like any pan) and marks it. A time past the
newest bar means the newest; a time before the oldest bar held goes to the oldest bar and reports
`Scrolled::BeforeHistory`. It only moves the viewport: no script reruns, no drawing changes, and
linked charts follow through the usual time-range sync. The Pine logs pane's "Scroll to bar" calls
it with the bar the message was written on (`LogEvent::bar_time_ms`), over the replay-cut candles
while a replay runs.

A bar older than the candles held is not paged for: loading older history makes the script run
from scratch, which would clear the very messages being browsed. The chart goes to its oldest bar
and says so; scrolling back from there loads history as usual.

On a price-based type (Renko, Kagi, Line break, Point & figure) the bars are the constructed ones,
which is also where scripts run and write their messages, so the scroll is over those. The scroll is
made outside a frame, so the card remembers where the view was (`scroll_from`) and the next frame
measures the change from there: that is how linked charts follow it.

The marker (`chart_card/bar_marker.rs`) is a vertical line with a tag at the time axis, in the accent
colour, for five seconds. Its label is the date for a daily or longer timeframe and the date and
time below. Like the locked cursor line it is view state, drawn by `paint_frame` and never saved,
never a drawing, never in a template. Not verified in a real window.

## Pine and indicators

Panning re-projects cached output; indicators are computed over the full series and sliced to the
window. A Pine script is run again on a pan only if its source reads `chart.left_visible_bar_time`
or `chart.right_visible_bar_time` (`reads_visible_range`); otherwise the visible range is not part
of its run revision. A chart with no scripts no longer hashes the series every frame.

## Verified and not verified

Verified by tests (`model/viewport/tests.rs`, `ui/chart_card/navigation_tests.rs`, `model/history`,
`workspace`): the pan maths, fractional offsets, bounded whitespace, vertical pan on linear and log
scales, autoscale versus manual range, axis drags and double-clicks, Escape, release outside the
chart, drawing ownership (handle, locked drawing, tool), a drawing's anchors unchanged and no
commands during a pan, live bars and older pages not moving the view, persistence, and script
reruns. A drag frame over 100,000 candles and 200 drawings costs about 0.12 ms headless in release
(`drag_frame_cost_report`, ignored).

**New-bar shifting** follows the Lightweight Charts source and docs; tradingview.com itself was not
compared.

**Not verified against TradingView.** TradingView's public docs do not state axis-drag direction,
double-click behavior, right-side whitespace limits, or when autoscale turns off. These are
inferred: dragging the body vertically turns autoscale off; dragging the price axis keeps it on
(the app's existing behavior); the time axis stretches with the newest bar fixed; whitespace is
bounded so that 20 bars remain on screen; the 4 pt vertical lock-in. Check them against the
product before calling any of them final.

## Not done

- **Per-pane vertical scales.** Script panes autoscale their own range every frame; a vertical drag
  in a pane does nothing. Horizontal navigation is shared by every pane.
- Price-based chart types navigate on their constructed bars' index, and "near the oldest bar" means
  the oldest constructed bar, so scrolling back there fetches older *source* candles and rebuilds.
  Not tested.
- **Replay** is untested end to end: replay cuts the bars the chart receives, so whitespace and
  stepping follow from the bar list, and a stepped-in bar adjusts the offset like a live one.
- Alert lines, paper-trading overlays and Pine drawings are projected through the same `View` as
  before; they were not exercised by panning tests beyond the shared geometry.
- Not driven in the running app: all of the above was exercised through egui's input pipeline in
  tests, not with a physical mouse.
