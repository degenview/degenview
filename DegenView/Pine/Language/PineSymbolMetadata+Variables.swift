import Foundation

extension PineSymbolMetadata {
    /// One-line documentation for the variables people reach for most. The rest show their type only.
    static let variableEntries = """
        open :: Opening price of the current bar.
        high :: Highest price of the current bar.
        low :: Lowest price of the current bar.
        close :: Closing price of the current bar; the last price on the realtime bar.
        volume :: Volume of the current bar.
        hl2 :: Average of high and low.
        hlc3 :: Average of high, low and close.
        ohlc4 :: Average of open, high, low and close.
        hlcc4 :: Average of high, low and twice the close.
        time :: UNIX time of the current bar's open, in milliseconds.
        time_close :: UNIX time of the current bar's close, in milliseconds.
        bar_index :: Index of the current bar, starting at 0.
        last_bar_index :: Index of the last chart bar.
        last_bar_time :: UNIX time of the last chart bar.
        timenow :: Current UNIX time, in milliseconds.
        chart.left_visible_bar_time :: UNIX time of the leftmost candle on the chart when the script was built.
        chart.right_visible_bar_time :: UNIX time of the rightmost candle on the chart when the script was built.
        ta.tr :: True range of the current bar.
        barstate.isfirst :: True on the first bar.
        barstate.islast :: True on the last bar.
        barstate.isconfirmed :: True when the bar has closed.
        barstate.isnew :: True on the first update of a bar.
        barstate.ishistory :: True on historical bars.
        barstate.isrealtime :: True on realtime bars.
        syminfo.tickerid :: Symbol id with its source prefix.
        syminfo.ticker :: Symbol name.
        syminfo.mintick :: Minimum price increment.
        syminfo.basecurrency :: The base asset of a pair, such as BTC for BTCUSDT; na when the symbol is not a pair.
        timeframe.period :: The chart timeframe as text.
        strategy.position_size :: Size of the open position; negative when short.
        strategy.equity :: Initial capital plus net and open profit.
        """
}
