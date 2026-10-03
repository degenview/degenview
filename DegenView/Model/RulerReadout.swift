import Foundation

/// What a ruler rectangle measures: the move as a percentage, the same move in price,
/// and how much chart it covers. Pure data so the numbers are testable apart from the
/// Canvas that draws them.
struct RulerReadout: Equatable {
    /// Signed move from the start anchor to the end anchor, in price.
    let priceDelta: Double
    /// `priceDelta` relative to the start price. Zero when the start price is zero.
    let percent: Double
    /// Candles covered, inclusive of the ones at both ends: a rectangle from one bar to
    /// the next covers two of them.
    let bars: Int
    /// Elapsed time between the anchors, always positive.
    let span: TimeInterval

    init(rect: RulerRect, points: [KlineData]) {
        priceDelta = rect.end.price - rect.start.price
        percent = rect.start.price != 0 ? priceDelta / rect.start.price * 100 : 0

        let startIndex = ChartPlot.fractionalIndex(of: rect.start.date, in: points).rounded()
        let endIndex = ChartPlot.fractionalIndex(of: rect.end.date, in: points).rounded()
        bars = Int(abs(endIndex - startIndex)) + 1
        span = abs(rect.end.date.timeIntervalSince(rect.start.date))
    }

    /// A flat move counts as up, matching `RulerRect.isUpward`.
    var isUpward: Bool { priceDelta >= 0 }

    private var sign: String { priceDelta < 0 ? "-" : "+" }

    /// "+4.23%". The sign is added here rather than left to the number formatter so a
    /// move that rounds to zero still reads with the direction it was measured in.
    var percentText: String {
        "\(sign)\(abs(percent).formatted(.number.precision(.fractionLength(2))))%"
    }

    /// "+1,234.50". `PriceFormatter` clamps probabilities to 0…100, so a downward move on
    /// a Polymarket chart would print as "0.0%" if the sign went through it — format the
    /// magnitude and prepend the sign.
    func priceText(decimalPlaces: Int?, scale: PriceScale) -> String {
        "\(sign)\(PriceFormatter.format(abs(priceDelta), decimalPlaces: decimalPlaces, scale: scale))"
    }

    var barsText: String { bars == 1 ? "1 bar" : "\(bars) bars" }

    /// Empty for a span under a minute, which the smallest candle still outlasts. Guarded
    /// here because the formatter would answer "0m".
    var durationText: String { span < 60 ? "" : TimeAxisFormatter.duration(span) }

    /// "23 bars · 1d 14h", or just the bars when there is no duration to report.
    var coverageText: String {
        durationText.isEmpty ? barsText : "\(barsText) · \(durationText)"
    }
}
