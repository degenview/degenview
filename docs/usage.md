# Usage

1. Click the toolbar **+** and choose Crypto, Stocks, Prediction Markets, CoinMarketCap, or
   Portfolio. Prediction Markets has a Polymarket | Kalshi switch; both work keylessly. Crypto search fans out across Binance, Coinbase, CoinGecko, and DEXScreener, listed in that order; stock
   search requires Alpaca keys in **Settings → Alpaca**. CoinMarketCap offers four
   market-wide index charts and works keylessly.
2. Pick a timeframe in the toolbar and scroll over a chart to zoom its history. Drag the
   price axis to zoom vertically.
3. Open a chart's gear menu to change its instrument, colors, decimal precision, and
   technical indicators. Use its **Scripts** tab to edit and apply a Pine v6-style
   indicator or change the generated inputs. The settings window can be resized.
4. Use the left tool strip for the synchronized crosshair, persistent trend lines,
   **Fib Retracement**, and the **Ruler**. Drag across the chart with the ruler to measure
   a move (or click, move, and click again); drag a corner to resize a measurement or an
   edge to move it, and press Delete to remove the selected one. Measurements are
   temporary. For a Fib, click once for Point
   1, move to preview its levels, and click again for Point 2. Select a completed Fib to
   drag its handles or body, open its settings, or delete it. Press Escape to cancel an
   incomplete drawing and Delete/Backspace to remove a selected drawing.
5. Drag cards to reorder them within or across columns. Hold a card at the grid's right edge until the outlined preview
   column appears, then release it to create that column. The option appears only when
   the window is wide enough to keep every resulting column readable.
6. Save the dashboard as a named view. Use the folder menu to load or delete views and to
   rename the current tab.
7. Toggle the Favorites sidebar, add instruments with its **+**, reorder them by dragging,
   and click one to open it in the current tab.
8. Press **⌘T** or use the tab bar's **+** for an empty tab. Drag tabs out into windows or
   merge them again through the tab bar or **File → Merge All Windows**.
9. Open the toolbar **Replay** menu and choose **Select Bar on Chart**. Move over a chart to
   snap the orange marker to a historical candle (the plot to its right dims), then click to
   begin; **Esc** or **Cancel** backs out. **Choose Date & Time…** opens a picker limited to
   the loaded chart. Use the replay strip to step forward or back (⇧→ / ⇧←), play or pause
   (⇧↓), drag the timeline to jump, change speed/resolution, choose a new start, or press
   **Live** to return to the latest data.
10. Open the toolbar **Portfolio** menu to create a dedicated Portfolio tab. Create a
    portfolio, add transactions manually, or choose **Import from CoinMarketCap**. Review
    automatic asset mappings and historical FX issues before committing the atomic import.
11. Use a chart card's bell to create an absolute or percentage price alert. Open the
    adjacent bell—or the Portfolio toolbar bell—to manage rules and history in the Alerts
    window (its **Script Alerts** filter lists Pine script alerts). Notification behavior is under
    **Settings → Notifications**.
12. Optionally add a CoinMarketCap key under **Settings → CoinMarketCap**. Save and remove
    operations use macOS Keychain, and open CMC charts adopt the new request mode without
    an application restart.

## Example Pine indicator

Paste this into a market chart's **Settings → Scripts** tab and click **Apply**. After it
compiles, the Fast EMA and Slow EMA controls appear below the editor and can be changed
without recompiling the source.

```pinescript
//@version=6
indicator("EMA Momentum", overlay=true)

fastLength = input.int(12, "Fast EMA", minval=1)
slowLength = input.int(26, "Slow EMA", minval=2)

fast = ta.ema(close, fastLength)
slow = ta.ema(close, slowLength)

bullish = ta.crossover(fast, slow)
bearish = ta.crossunder(fast, slow)

plot(fast, color=color.orange, linewidth=2)
plot(slow, color=color.blue, linewidth=2)

plotshape(bullish, color=color.green)
plotshape(bearish, color=color.red)
```

The Scripts tab in chart settings loads a saved script onto the chart: pick one from **Script**,
or **None** to unload it. Scripts are edited only in the Script Manager; the tab's **Script
Manager** button opens it in a new tab, and saving a script there updates charts using it.

The Script Manager also runs the script you are editing. A live chart sits beside the code (use
the toolbar's position menu to put it left of, above or below the editor, or hide it with ⌥⌘P):
it re-runs a moment after you stop typing, and while the code does not compile it keeps showing
the last working version. Pick the market from the header — crypto or US stocks (stocks need
Alpaca keys in Settings) — and a timeframe, scroll over the chart to zoom, and drag its price
axis to make the candles taller or shorter (double-click resets). The bar under the chart opens **Inputs** (every `input.*` declaration as a control; values are remembered per
script, **Reset to Defaults** clears them), **Report** (strategy results and alerts) and
**Problems**. Hide the script list with ⌃⌘S. Previews never raise alerts.

Scripts declared with `strategy()` are backtested over the chart's bars: the Scripts tab
shows net profit, win rate, profit factor, drawdown, an equity curve, and the trade list,
and the chart marks each entry and exit. `alert()` and `alertcondition()` calls are listed
there too. To be notified when a script raises one on a live bar, choose **Create Alert…** in the
Scripts tab: you get a macOS notification and an in-app banner, and the alert appears under
**Script Alerts** in the Alerts window. It fires only while that chart shows the same symbol and
timeframe, and editing the script pauses it until you re-arm it there.

The engine supports a subset of Pine rather than every feature. See
[Pine compatibility](pine-compatibility.md) for exact syntax, built-ins, limits, and known
differences.

## Replay data support

| Provider | Granular replay | Available behavior |
| --- | --- | --- |
| Binance | Yes | `1m`, `5m`, `15m`, `30m`, `1h`, and `1D` where finer than the chart |
| Coinbase | Yes | `1m`, `5m`, `15m`, `1h`, and `1D` where finer than the chart (no 30m candles) |
| Alpaca | Yes | Minute/hour/day historical bars from the configured IEX feed |
| CoinGecko | No | Complete displayed bars |
| DEXScreener / GeckoTerminal | No | Complete displayed bars |
| Polymarket / Kalshi | No | Complete displayed observations |
| CoinMarketCap indices | No | Dedicated index history and latest-value widgets; not OHLCV |

**Auto** chooses the finest provider-supported interval that fits the loaded span within
the 100,000-source-bar replay budget. Intervals that cannot cover the span accurately are
not shown. If a granular request fails or contains no data, that chart displays a
non-blocking notice and safely falls back to complete bars.
