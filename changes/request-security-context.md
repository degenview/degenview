fixed: Pine scripts that use timeframe.change no longer see a change on the first bar of the chart, or on the first candle of a request.security series, as on TradingView.
fixed: Inside request.security, bar_index now starts at 0, so barstate.isfirst is true on the first requested candle, as on TradingView.
fixed: barstate.ishistory, barstate.isrealtime, barstate.islast, barstate.isnew and barstate.isconfirmed inside request.security now follow the chart bar being drawn, as on TradingView, so scripts that act on the last or latest bar of a request behave the same.
fixed: On a monthly chart, request.security to a quarterly or yearly timeframe now returns the finished quarter or year on its last month, as on TradingView, instead of one month late.
