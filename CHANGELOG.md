# Changelog

What changed for people who use DegenView, release by release, newest first. `tools/release.sh` writes
each section from the fragments in `changes/` when it cuts the release, and the app shows the
sections between your version and the one on offer before it updates.

## 0.6.0-rc.35 (2026-11-24)

### New

- Charts can use second timeframes (1s, 15s, 30s and 45s in the timeframe menu, or any 1-59s you type) on Binance and OKX, and 10s and 30s on Gate.io.

### Fixed

- Binance markets were missing from search, which showed "Binance could not be searched" instead.
- Pine scripts that change a line, label or box drawn at a time (xloc.bar_time) no longer corrupt its price, width or size when they set them as whole numbers.
- Pine labels, boxes and table cells set in a monospace font are now drawn in one, and bold or italic label text is warned about instead of silently ignored.
- Pine tables and labels are drawn at TradingView's sizes: large and huge text was far too small, and a label's bubble now grows with its text size.
- RSI now gets a pane of its own with its own scale, like MACD, instead of being drawn over the candles. Charts that already had RSI open with it in a pane.
- With Keep drawing off, clicking after a Ruler measurement now puts it away and returns to the crosshair.
- Scripts on a chart now run over up to 5,000 candles of history, as on TradingView, so levels, sessions and crossovers that need days of data are no longer missing.

### Fixed in release candidates

- One click on the cursor tile in the chart's left toolbar picks the cursor again; its menu opens from the tile's corner or a right-click, like the other tiles.

## 0.6.0-rc.34 (2026-11-23)

### New

- Drag the line above a script pane to resize it, as in TradingView; pane heights are saved with the chart.

### Fixed

- The app no longer fails to open without a message on some Windows PCs; it starts with DirectX 12 and switches to another graphics mode by itself if a start does not work.

### Fixed in release candidates

- Text no longer looks tall and stretched on screens that are not Retina.
- The update sheet no longer says its release notes are complete when it stopped reading the release list early.

## 0.6.0-rc.33 (2026-11-21)

### New

- The Update button now shows what is new and what was fixed before you install.

### Fixed

- Closing the last window on Linux or Windows no longer deletes your tabs and drawings
- Pine scripts with loops now show the right values when a loop runs a different number of times from bar to bar.
- On Wayland (GNOME, KDE), closing Settings and opening new tabs no longer wait until the window is shown again, and no empty extra DegenView window appears in Alt+Tab

### Fixed in release candidates

- US stock charts on Alpaca's free plan load the latest bars again instead of showing a data error.
