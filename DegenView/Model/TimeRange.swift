import Foundation

enum TimeRange: String, CaseIterable, Identifiable, Codable {
    case oneHour = "1H"
    case oneDay = "1D"
    case oneWeek = "1W"
    case oneMonth = "1M"
    case threeMonths = "3M"
    case oneYear = "1Y"

    var id: String { rawValue }

    /// Candle interval token — directly matches the picker label, so each timeframe is the size
    /// of one candle. `3M` and `1Y` have no native kline anywhere; each provider builds them by
    /// folding monthly or daily candles into calendar quarters and years.
    var binanceInterval: String {
        switch self {
        case .oneHour: return "1h"
        case .oneDay: return "1d"
        case .oneWeek: return "1w"
        case .oneMonth: return "1M"
        case .threeMonths: return "3M"
        case .oneYear: return "1Y"
        }
    }

    /// Duration of one ``binanceInterval`` candle in seconds.
    ///
    /// Used to compute the effective span a Binance chart covers at this range.
    var binanceIntervalSeconds: TimeInterval {
        switch self {
        case .oneHour: return 3_600  // 1h
        case .oneDay: return 86_400  // 1d
        case .oneWeek: return 604_800  // 1w
        case .oneMonth: return 2_592_000  // 1M (30d)
        case .threeMonths: return 7_776_000  // 3M (90d)
        case .oneYear: return 31_536_000  // 1Y (365d)
        }
    }

    /// Time span the prediction-market line charts cover at this range, in days.
    ///
    /// They draw a price line rather than candles, so a quarterly or yearly candle size means
    /// nothing there: 3M and 1Y stay windows of three months and a year.
    var effectiveSpanDays: Int {
        switch self {
        case .oneHour: return 2
        case .oneDay: return 60
        case .oneWeek: return 182
        case .oneMonth: return 360
        case .threeMonths: return 90
        case .oneYear: return 364
        }
    }

    /// Points a prediction-market line chart draws at this range. Independent of
    /// ``dataPointLimit``: a line of 10 points for a year would be a polyline, not a chart.
    var lineChartPointCount: Int {
        switch self {
        case .oneHour: return 48
        case .oneDay: return 60
        case .oneWeek: return 26
        case .oneMonth: return 12
        case .threeMonths: return 90
        case .oneYear: return 52
        }
    }

    /// Polymarket CLOB `/prices-history` window: named interval plus the bucket
    /// size in minutes.
    ///
    /// The interval is picked to match ``effectiveSpanDays`` as closely as the
    /// Polymarket API allows. Fidelity is the coarsest bucket that still yields
    /// at least `dataPointLimit × 2` raw points — enough headroom for the
    /// downsampling step.
    ///
    /// The endpoint returns an empty array when `intervalMinutes / fidelity`
    /// exceeds a few thousand points, so fidelity can't be arbitrarily fine.
    /// Polymarket retains roughly a month of history per market, so spans beyond
    /// ~30 days all land on `"max"` and render whatever exists.
    var polymarketWindow: (interval: String, fidelity: Int) {
        let span = effectiveSpanDays
        let needed = lineChartPointCount * 2

        let interval: String
        if span <= 1 {
            interval = "1h"
        } else if span <= 2 {
            interval = "1d"
        } else if span <= 8 {
            interval = "1w"
        } else if span <= 31 {
            interval = "1m"
        } else {
            interval = "max"
        }

        let intervalMinutes: Double =
            interval == "max"
            ? 43_200  // ~30 days
            : interval == "1m"
                ? 43_200
                : interval == "1w"
                    ? 10_080
                    : interval == "1d"
                        ? 1_440
                        : 60  // "1h"

        let fidelity = max(1, Int(intervalMinutes / Double(needed)))

        return (interval, fidelity)
    }

    /// Kalshi candlestick window: period in minutes (the API only accepts 1, 60 and
    /// 1440) and how far back to start.
    ///
    /// Kalshi rejects a request that would return more than roughly 5,000 candles,
    /// so the period steps up with the span: 1-minute bars for ≤ 2 days, hourly to
    /// 120 days, daily beyond.
    var kalshiWindow: (periodMinutes: Int, spanSeconds: TimeInterval) {
        let span = effectiveSpanDays
        let period = span <= 2 ? 1 : (span <= 120 ? 60 : 1_440)
        return (period, TimeInterval(span) * 86_400)
    }

    /// Number of candles to fetch from the API.
    var dataPointLimit: Int {
        switch self {
        case .oneHour: return 48  // 2 days of 1h candles
        case .oneDay: return 60  // ~2 months of 1d candles
        case .oneWeek: return 26  // ~6 months of 1w candles
        case .oneMonth: return 12  // 1 year of 1M candles
        case .threeMonths: return 16  // 4 years of quarterly candles
        case .oneYear: return 10  // 10 years of yearly candles
        }
    }

    /// The candle count earlier versions stored for this range. A tab or saved view still
    /// holding one of these was never zoomed — it predates the range meaning a candle size —
    /// and would draw 90 quarters across a history that is a dozen long.
    func migratedCandleCount(_ stored: Int) -> Int {
        switch (self, stored) {
        case (.threeMonths, 90), (.oneYear, 52): return dataPointLimit
        default: return stored
        }
    }

}
