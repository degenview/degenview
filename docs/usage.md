# Usage

1. Click the toolbar **+** and choose Crypto, Stocks, Polymarket, CoinMarketCap, or
   Portfolio. Crypto search fans out across Binance, CoinGecko, and DEXScreener; stock
   search requires Alpaca keys in **Settings → Alpaca**. CoinMarketCap offers four
   market-wide index charts and works keylessly.
2. Pick a timeframe in the toolbar and scroll over a chart to zoom its history. Drag the
   price axis to zoom vertically.
3. Open a chart's gear menu to change its instrument, colors, decimal precision, and
   technical indicators. Use its **Scripts** tab to edit and apply a Pine v6-style
   indicator or change the generated inputs. The settings window can be resized.
4. Use the left tool strip for the synchronized crosshair, persistent trend lines,
   **Fib Retracement**, and temporary ruler measurements. For a Fib, click once for Point
   1, move to preview its levels, and click again for Point 2. Select a completed Fib to
   drag its handles or body, open its settings, or delete it. Press Escape to cancel an
   incomplete drawing and Delete/Backspace to remove a selected drawing.
5. Switch between the vertical and grid layouts, then drag cards to reorder them within
   or across columns. Hold a card at the grid's right edge until the outlined preview
   column appears, then release it to create that column. The option appears only when
   the window is wide enough to keep every resulting column readable.
6. Save the dashboard as a named view. Use the folder menu to load or delete views and to
   rename the current tab.
7. Toggle the Favorites sidebar, add instruments with its **+**, reorder them by dragging,
   and click one to open it in the current tab.
8. Press **⌘T** or use the tab bar's **+** for an empty tab. Drag tabs out into windows or
   merge them again through the tab bar or **File → Merge All Windows**.
9. Open the toolbar **Replay** menu and choose **Select bar**. Move over a chart to snap
   the orange marker to a historical candle, then click to begin. Use the replay strip to
   step, play, change speed/resolution, choose a new start, or return to latest.
10. Open the toolbar **Portfolio** menu to create a dedicated Portfolio tab. Create a
    portfolio, add transactions manually, or choose **Import from CoinMarketCap**. Review
    automatic asset mappings and historical FX issues before committing the atomic import.
11. Use a chart card's bell to create an absolute or percentage price alert. Open the
    adjacent bell—or the Portfolio toolbar bell—to manage rules and history in the Alerts
    window. Notification behavior is under **Settings → Notifications**.
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

The current engine intentionally supports an indicator-focused subset rather than every
Pine feature. See [Pine compatibility](pine-compatibility.md) for exact syntax, built-ins,
limits, and known differences.

## Replay data support

| Provider | Granular replay | Available behavior |
| --- | --- | --- |
| Binance | Yes | `1m`, `5m`, `15m`, `30m`, `1h`, and `1D` where finer than the chart |
| Alpaca | Yes | Minute/hour/day historical bars from the configured IEX feed |
| CoinGecko | No | Complete displayed bars |
| DEXScreener / GeckoTerminal | No | Complete displayed bars |
| Polymarket | No | Complete displayed observations |
| CoinMarketCap indices | No | Dedicated index history and latest-value widgets; not OHLCV |

**Auto** chooses the finest provider-supported interval that fits the loaded span within
the 100,000-source-bar replay budget. Intervals that cannot cover the span accurately are
not shown. If a granular request fails or contains no data, that chart displays a
non-blocking notice and safely falls back to complete bars.
