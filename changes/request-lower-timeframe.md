fixed: request.security to a finer timeframe than the chart now returns the first intrabar of each chart bar with lookahead_on, and its barstate.islast, islastconfirmedhistory and isrealtime follow the requested series as on TradingView.
fixed: A request.security to a finer timeframe on a stock or monthly chart now reads the candles that end inside each chart bar, so the last candle of a day or of a 31-day month is no longer counted in the next bar.
fixed: barstate.islast inside request.security is now true for the series' latest candle on every chart bar that reads it, not only on the chart's last bar.
fixed: On a monthly, quarterly or yearly chart, request.security to the chart's own timeframe is no longer treated as a finer timeframe when the latest month has 31 days, so its values match TradingView.
fixed: On a monthly chart with a live bar, last_bar_time is now the live bar's real open, a quarterly request no longer marks the finished quarter as the last one, and time_close of a month is the start of the next month as on TradingView.
fixed: Scripts that request a finer timeframe on a monthly chart now receive the last day of a 31-day month.
