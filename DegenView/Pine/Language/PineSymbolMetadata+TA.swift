import Foundation

extension PineSymbolMetadata {
    /// `ta.*`: every name the runtime computes.
    static let taEntries = """
        ta.sma(source: series float, length: series int) -> series float :: Simple moving average.
        ta.ema(source: series float, length: simple int) -> series float :: Exponential moving average.
        ta.rma(source: series float, length: simple int) -> series float :: Running moving average, as used by RSI.
        ta.wma(source: series float, length: series int) -> series float :: Weighted moving average.
        ta.hma(source: series float, length: series int) -> series float :: Hull moving average.
        ta.swma(source: series float) -> series float :: Symmetrically weighted moving average over four bars.
        ta.rsi(source: series float, length: simple int) -> series float :: Relative Strength Index.
        ta.cmo(source: series float, length: series int) -> series float :: Chande Momentum Oscillator.
        ta.cci(source: series float, length: series int) -> series float :: Commodity Channel Index.
        ta.mom(source: series float, length: series int) -> series float :: Momentum: source minus source length bars ago.
        ta.roc(source: series float, length: series int) -> series float :: Rate of change in percent.
        ta.change(source: series float, length: series int = 1) -> series float :: Difference from length bars ago.
        ta.stdev(source: series float, length: series int, biased: series bool = true) -> series float :: Standard deviation.
        ta.variance(source: series float, length: series int, biased: series bool = true) -> series float :: Variance.
        ta.dev(source: series float, length: series int) -> series float :: Mean absolute deviation.
        ta.median(source: series float, length: series int) -> series float :: Median over the last length bars.
        ta.range(source: series float, length: series int) -> series float :: Highest minus lowest over the last length bars.
        ta.percentrank(source: series float, length: series int) -> series float :: Percent of earlier values at or below the current one.
        ta.highest(source: series float, length: series int) -> series float :: Highest value over the last length bars.
        ta.highest(length: series int) -> series float :: Highest high over the last length bars.
        ta.lowest(source: series float, length: series int) -> series float :: Lowest value over the last length bars.
        ta.lowest(length: series int) -> series float :: Lowest low over the last length bars.
        ta.highestbars(source: series float, length: series int) -> series int :: Bars since the highest value, as a negative offset.
        ta.highestbars(length: series int) -> series int :: Bars since the highest high, as a negative offset.
        ta.lowestbars(source: series float, length: series int) -> series int :: Bars since the lowest value, as a negative offset.
        ta.lowestbars(length: series int) -> series int :: Bars since the lowest low, as a negative offset.
        ta.max(source: series float) -> series float :: Highest value of source since the first bar.
        ta.min(source: series float) -> series float :: Lowest value of source since the first bar.
        ta.rising(source: series float, length: series int) -> series bool :: True when source rose on each of the last length bars.
        ta.falling(source: series float, length: series int) -> series bool :: True when source fell on each of the last length bars.
        ta.cross(source1: series float, source2: series float) -> series bool :: True when the two series cross in either direction.
        ta.crossover(source1: series float, source2: series float) -> series bool :: True when source1 crosses above source2.
        ta.crossunder(source1: series float, source2: series float) -> series bool :: True when source1 crosses below source2.
        ta.macd(source: series float, fastlen: simple int, slowlen: simple int, siglen: simple int) -> [series float, series float, series float] :: MACD line, signal line and histogram.
        ta.atr(length: simple int) -> series float :: Average true range.
        ta.tr(handle_na: simple bool = false) -> series float :: True range of the current bar.
        ta.bb(series: series float, length: series int, mult: simple float) -> [series float, series float, series float] :: Bollinger Bands: basis, upper and lower.
        ta.dmi(diLength: simple int, adxSmoothing: simple int) -> [series float, series float, series float] :: Directional movement: +DI, -DI and ADX.
        ta.sar(start: simple float, inc: simple float, max: simple float) -> series float :: Parabolic SAR.
        ta.linreg(source: series float, length: series int, offset: simple int) -> series float :: Linear regression value.
        ta.correlation(source1: series float, source2: series float, length: series int) -> series float :: Correlation coefficient of two series.
        ta.cum(source: series float) -> series float :: Cumulative sum of source.
        ta.barssince(condition: series bool) -> series int :: Bars since the condition was last true.
        ta.pivothigh(source: series float, leftbars: series int, rightbars: series int) -> series float :: Price of a pivot high, or na.
        ta.pivothigh(leftbars: series int, rightbars: series int) -> series float :: Price of a pivot high of the high series, or na.
        ta.pivotlow(source: series float, leftbars: series int, rightbars: series int) -> series float :: Price of a pivot low, or na.
        ta.pivotlow(leftbars: series int, rightbars: series int) -> series float :: Price of a pivot low of the low series, or na.
        ta.vwap(source: series float, anchor: series bool = na, stdev_mult: simple float = na) -> series float :: Volume-weighted average price.
        """
}
