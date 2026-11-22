fixed: Pine scripts that read a timeframe the chart's bars do not divide (a 45 minute candle on a 30 minute chart) now get the exchange's own candles instead of a fold that cut through the chart's bars.
fixed: Pine scripts that read another timeframe now honour calc_bars_count, and a chart that starts in the middle of a higher-timeframe candle reads that first candle whole.
