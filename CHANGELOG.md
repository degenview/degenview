# Changelog

What changed for people who use DegenView, release by release, newest first. `tools/release.sh` writes
each section from the fragments in `changes/` when it cuts the release, and the app shows the
sections between your version and the one on offer before it updates.

## 0.6.0-rc.38 (2026-11-27)

### New

- Binance USDⓈ-M perpetual futures can be charted, such as BTCUSDT.P, with live candles and their own order book.
- The symbol picker names each market's asset in its own column and shows a chart of its last day beside the price.
- Import your layouts, drawings, watchlists and Pine scripts from the TradingView desktop app; DegenView offers to when it first finds it.

### Fixed

- Delete, Backspace, Enter and the replay keys no longer act on what is behind an open dialog.
- A new script no longer replaces a script file whose name differs only in how its accents are stored.
- Binance and Bybit markets such as SUSDT, STRY or BTCMXN now load, with live prices.
- A Pine script that runs too long on a method call now says so instead of reporting an unknown function.
- A history (`[1]`) of `ta.tr`, `hlcc4`, `dayofweek` and the other builtin variables the engine computes now reads the requested series' previous bar inside `request.security`, as on TradingView, instead of `na`.
- Pine scripts can read objects stored in maps and matrices returned from another symbol or timeframe without losing their values.
- A script alert whose script no longer compiles, or stops with a runtime error, now says so, with the error, instead of showing as active and never firing.
- A script alert on a tab that is not on screen now fires, instead of showing as active while nothing reaches it.
- A script alert is no longer switched off by the charts of another tab.
- Re-arm on a script alert now arms it on the script as its chart has it now, instead of switching straight back to "Script changed".
- A script name with characters like | or ? is saved with - in their place instead of being refused or failing on Windows.
- A script that stops with a runtime error on a live bar stays stopped, instead of starting over on the next price update.
- A connection test in Settings no longer shows its result after you change what it tested.
- A Pine strategy that passes its own state (strategy.equity, strategy.position_size and the like), or anything computed from it, to request.security or another request is now refused with an error, as on TradingView, instead of silently reading an empty account.

### Fixed in release candidates

- The language list in Settings shows 简体中文 instead of empty boxes when the app is in another language.
- Oanda practice accounts connect: choose Live or Practice in Settings ▸ Oanda.

## 0.6.0-rc.37 (2026-11-27)

### New

- DegenView is available in German. Pick it, or leave it on the system's language, in Settings ▸ Appearance ▸ Language; a restart applies the choice.
- Charts, the symbol picker and watchlists can use Oanda forex, metals and index CFDs once an account ID and access token are saved in Settings ▸ Oanda.
- DegenView is available in Russian. Pick it, or leave it on the system's language, in Settings ▸ Appearance ▸ Language; a restart applies the choice.

### Fixed

- Pine boxes that extend left or right now reach the edge of the chart however far back they start, and their text stays by the box.
- Fills between Pine lines that both extend now run on with them.
- Pine lines drawn from right to left, and upright ones, now extend the same way as on TradingView.
- An unfilled market order shows “Market” as its price in Spanish and Chinese, not a dash.
- The CoinMarketCap Fear & Greed and Altcoin Season cards name the reading in the interface language, not in English.
- Two, three and four week candles (and unusual month lengths such as 5M) now start where TradingView's do, counted from the start of each year. Existing charts on these timeframes show different candles.
- Shortened Russian footer hints in the timeframe picker so the popover stays compact.
- On Windows and Linux, settings and messages no longer call your computer a Mac or mention the Keychain.
- Pine's timeframe.change now matches TradingView on stock charts (also around holidays and inside request.security of another market), when the timeframe differs from one bar to the next, and with an empty timeframe.

### Fixed in release candidates

- On a stock chart, barstate.islastconfirmedhistory inside request.security is true again on the last finished week or month, as on TradingView.

## 0.6.0-rc.36 (2026-11-26)

### New

- The last price on the axis can show how long the current bar has left before it closes. Turn it on or off with Countdown to bar close, in the price axis menu or in the chart settings.
- Drag rows in the object tree up and down to change the order of drawings, compared symbols and scripts on the chart.
- DegenView is available in Simplified Chinese. Pick it, or leave it on the system's language, in Settings ▸ Appearance ▸ Language; a restart applies the choice.
- DegenView is available in Spanish (Spain). Pick it, or leave it on the system's language, in Settings ▸ Appearance ▸ Language; a restart applies the choice.

### Fixed

- A Pine script that reads a built-in function without calling it (`plot(ta.sma)`, `x = nz`) now shows a compile error on that line, as on TradingView, instead of stopping at run time with "Undefined variable".
- A history (`[1]`) of a call inside a ternary, `and`/`or`, `switch` or `if` operand in a Pine script now reads the value the call has on every bar, as on TradingView.
- Pine scripts can loop directly over a map’s keys and values without a runtime error.
- Pine `map.put_all` copies the pairs of the other map instead of failing with "requires a map".
- request.security to a finer timeframe than the chart now returns the first intrabar of each chart bar with lookahead_on, and its barstate.islast, islastconfirmedhistory and isrealtime follow the requested series as on TradingView.
- A request.security to a finer timeframe on a stock or monthly chart now reads the candles that end inside each chart bar, so the last candle of a day or of a 31-day month is no longer counted in the next bar.
- barstate.islast inside request.security is now true for the series' latest candle on every chart bar that reads it, not only on the chart's last bar.
- On a monthly, quarterly or yearly chart, request.security to the chart's own timeframe is no longer treated as a finer timeframe when the latest month has 31 days, so its values match TradingView.
- On a monthly chart with a live bar, last_bar_time is now the live bar's real open, a quarterly request no longer marks the finished quarter as the last one, and time_close of a month is the start of the next month as on TradingView.
- Scripts that request a finer timeframe on a monthly chart now receive the last day of a 31-day month.
- Inside request.security, bar_index now starts at 0, so barstate.isfirst is true on the first requested candle, as on TradingView.
- barstate.ishistory, barstate.isrealtime, barstate.islast, barstate.isnew and barstate.isconfirmed inside request.security now follow the chart bar being drawn, as on TradingView, so scripts that act on the last or latest bar of a request behave the same.
- On a monthly chart, request.security to a quarterly or yearly timeframe now returns the finished quarter or year on its last month, as on TradingView, instead of one month late.
- Right-clicking a chart while a drawing tool or Zoom In is selected switches back to the cursor; right-click again for the menu.
- Searching for a name with spaces, symbols or non-English characters (比特币, café, "a & b") no longer fails on DEXScreener, CoinGecko and Polymarket.
- US stock charts of 1h and longer intraday timeframes now cut their candles on the regular session like TradingView does: 4h is 09:30 and 13:30 New York, the last candle of a day closes at 16:00, and pre-market and after-hours candles are no longer drawn.
- Pine scripts on a stock that request a higher timeframe (a 4h, a day, a week) now get that candle when the session closes, where TradingView gives it, instead of at midnight.
- `[line, direction] = ta.supertrend(3, 10)` in Pine scripts no longer reads na on every bar; it matches TradingView from the first bar. Scripts that saved the old result may plot and trade differently.
- Pine scripts that use `ta.supertrend`, `ta.macd`, `ta.bb`, `ta.dmi` or `ta.kc` as a single value, or name the wrong number of values, now show a compile error as on TradingView instead of silently plotting nothing.
- ta.cmo, ta.cci, ta.mfi and ta.stoch now give TradingView's values when the price has not moved over their window, and ta.mfi starts one bar earlier. Saved scripts that use them plot and trade differently.
- ta.percentile_linear_interpolation and array.percentile_linear_interpolation now interpolate like TradingView, and never go beyond the lowest and highest value. Saved scripts that use them plot and trade differently.
- ta.pivot_point_levels with the Woodie type and developing = true now stops the script with an error, like on TradingView, instead of drawing levels.
- `ta.stdev(source, length, false)` and `ta.variance(source, length, false)` in Pine scripts now give the sample figure (divided by n - 1) instead of ignoring `biased`, as on TradingView.
- `ta.bbw` is now in percent, `ta.iii` is the value of each bar instead of a running total, and `ta.obv`, `ta.pvt` and `ta.cum` start on the first bar that has a value, as on TradingView. Scripts that use them may plot and trade differently.
- ta.ema, ta.rma, ta.rsi, ta.wma, ta.stdev, ta.variance, ta.median, ta.range, math.sum and Bollinger bands now treat a missing (na) bar in the source like TradingView, instead of going blank for a whole window or restarting. Saved scripts that use them plot and trade differently.
- ta.tr, ta.tr(false), the Keltner bands and DMI start one bar later, like on TradingView, and ta.tr(true) and a conditional ta.atr, ta.supertrend, ta.kc or ta.dmi measure the range against the chart's previous close. Saved scripts that use them plot and trade differently.
- Pine's timeframe.change now compares with the last bar its own call ran on, so a call inside an if, a condition or a function counts the same changes as TradingView, and the very first call is false.
- A tool button on the left strip now arms its tool on a click and opens its sub-tool menu only from its arrow, or on a click when its tool is already selected, as on TradingView; Escape shuts an open menu first.
- `ta.vwap` can be read as a variable in a Pine script (`plot(ta.vwap)`) instead of stopping the script with "Undefined variable"; it equals `ta.vwap(hlc3)`, as on TradingView.

### Fixed in release candidates

- Charts no longer crash when formatting some prices just below 1.

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
