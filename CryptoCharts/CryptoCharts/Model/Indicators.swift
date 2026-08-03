import Foundation

// MARK: - Series

/// Indicator readings for one chart, each array aligned 1:1 with the *visible*
/// candles.
///
/// Built from the full fetched buffer and then trimmed to the visible tail, so a
/// 200-period EMA is already warmed up at the left edge of the chart instead of
/// starting 200 candles in. Empty arrays mean the indicator is switched off.
struct IndicatorSeries {
    var rsi: [Double?] = []
    var ema: [Double?] = []
    var emaPeriod: Int = Indicator.emaDefaultPeriod
    var bollinger: BollingerSeries?

    static let none = IndicatorSeries()

    /// Compute every enabled indicator over `candles`, then trim to the last
    /// `visibleCount` readings.
    static func make(
        candles: [KlineData],
        visibleCount: Int,
        showRSI: Bool,
        showEMA: Bool,
        emaPeriod: Int,
        showBollinger: Bool
    ) -> IndicatorSeries {
        guard !candles.isEmpty else { return .none }

        func visible(_ values: [Double?]) -> [Double?] {
            guard visibleCount > 0, values.count > visibleCount else { return values }
            return Array(values.suffix(visibleCount))
        }

        var series = IndicatorSeries()
        series.emaPeriod = emaPeriod

        if showRSI {
            series.rsi = visible(candles.rsi())
        }
        if showEMA {
            series.ema = visible(candles.ema(period: emaPeriod))
        }
        if showBollinger {
            let bands = candles.bollingerBands()
            series.bollinger = BollingerSeries(
                upper: visible(bands.upper),
                middle: visible(bands.middle),
                lower: visible(bands.lower)
            )
        }
        return series
    }
}

/// Upper / middle / lower Bollinger bands, each aligned with the visible candles.
struct BollingerSeries {
    let upper: [Double?]
    let middle: [Double?]
    let lower: [Double?]

    var hasValues: Bool { middle.contains { $0 != nil } }
}

// MARK: - Math

extension Array where Element == KlineData {

    /// Relative Strength Index.
    ///
    /// Uses Wilder's smoothing (a modified EMA with α = 1/period) rather than a plain
    /// moving average — the naive version drifts noticeably from what every other
    /// charting tool shows. The first `period` entries are nil: RSI has no value until
    /// there are that many changes to average.
    func rsi(period: Int = RSI.period) -> [Double?] {
        // Spelled out: inside an extension on Array, a bare `Array(...)` resolves to Self.
        guard period > 0, count > period else { return [Double?](repeating: nil, count: count) }

        var values = [Double?](repeating: nil, count: count)

        // Seed from the simple average of the first `period` changes, then smooth.
        var averageGain = 0.0
        var averageLoss = 0.0
        for i in 1...period {
            let change = self[i].closePrice - self[i - 1].closePrice
            if change >= 0 { averageGain += change } else { averageLoss -= change }
        }
        averageGain /= Double(period)
        averageLoss /= Double(period)
        values[period] = rsiFrom(gain: averageGain, loss: averageLoss)

        guard count > period + 1 else { return values }

        let weight = Double(period - 1)
        for i in (period + 1)..<count {
            let change = self[i].closePrice - self[i - 1].closePrice
            averageGain = (averageGain * weight + Swift.max(change, 0)) / Double(period)
            averageLoss = (averageLoss * weight + Swift.max(-change, 0)) / Double(period)
            values[i] = rsiFrom(gain: averageGain, loss: averageLoss)
        }

        return values
    }

    /// Exponential moving average of closes, seeded with the simple average of the
    /// first `period` closes. Entries before that seed are nil.
    func ema(period: Int) -> [Double?] {
        guard period > 0, count >= period else { return [Double?](repeating: nil, count: count) }

        var values = [Double?](repeating: nil, count: count)

        var seed = 0.0
        for i in 0..<period { seed += self[i].closePrice }
        seed /= Double(period)
        values[period - 1] = seed

        let alpha = 2 / (Double(period) + 1)
        var previous = seed
        for i in period..<count {
            previous = self[i].closePrice * alpha + previous * (1 - alpha)
            values[i] = previous
        }

        return values
    }

    /// Bollinger bands: a simple moving average with a band `multiplier` population
    /// standard deviations either side, both measured over the same window.
    ///
    /// Population rather than sample deviation (divide by N, not N-1) — that's what
    /// Bollinger specified and what other tools draw.
    func bollingerBands(
        period: Int = Indicator.bollingerPeriod,
        multiplier: Double = Indicator.bollingerMultiplier
    ) -> (upper: [Double?], middle: [Double?], lower: [Double?]) {
        let blank = [Double?](repeating: nil, count: count)
        guard period > 1, count >= period else { return (blank, blank, blank) }

        var upper = blank, middle = blank, lower = blank
        let window = Double(period)

        // Rolling sums, so the cost doesn't grow with the period.
        var sum = 0.0
        var sumOfSquares = 0.0
        for i in 0..<count {
            let close = self[i].closePrice
            sum += close
            sumOfSquares += close * close

            if i >= period {
                let dropped = self[i - period].closePrice
                sum -= dropped
                sumOfSquares -= dropped * dropped
            }
            guard i >= period - 1 else { continue }

            let mean = sum / window
            // Clamped at zero: floating-point drift can leave the variance a hair
            // below it when every close in the window is identical.
            let variance = Swift.max(sumOfSquares / window - mean * mean, 0)
            let deviation = variance.squareRoot() * multiplier

            middle[i] = mean
            upper[i] = mean + deviation
            lower[i] = mean - deviation
        }

        return (upper, middle, lower)
    }
}

/// One RSI reading. A run with no losses pins to 100 (or to the neutral 50 when the
/// series hasn't moved at all), which is where the ratio would otherwise divide by zero.
private func rsiFrom(gain: Double, loss: Double) -> Double {
    guard loss > 0 else { return gain > 0 ? 100 : 50 }
    return 100 - 100 / (1 + gain / loss)
}
