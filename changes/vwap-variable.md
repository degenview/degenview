fixed: `ta.vwap` can be read as a variable in a Pine script (`plot(ta.vwap)`) instead of stopping the script with "Undefined variable"; it equals `ta.vwap(hlc3)`, as on TradingView.
