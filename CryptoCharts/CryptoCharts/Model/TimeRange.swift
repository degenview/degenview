import Foundation

enum TimeRange: String, CaseIterable, Identifiable, Codable {
    case oneHour = "1H"
    case oneDay = "1D"
    case oneWeek = "1W"
    case oneMonth = "1M"
    case threeMonths = "3M"
    case oneYear = "1Y"

    var id: String { rawValue }

    /// Binance kline interval string — directly matches the picker label.
    var binanceInterval: String {
        switch self {
        case .oneHour:      return "1h"
        case .oneDay:       return "1d"
        case .oneWeek:      return "1w"
        case .oneMonth:     return "1M"
        case .threeMonths:  return "1d"    // no "3M" on Binance; daily bars for 3 months
        case .oneYear:      return "1w"    // no "1Y" on Binance; weekly bars for 1 year
        }
    }

    /// Duration of one ``binanceInterval`` candle in seconds.
    ///
    /// Used to compute the effective span a Binance chart covers at this range.
    var binanceIntervalSeconds: TimeInterval {
        switch self {
        case .oneHour:      return 3_600       // 1h
        case .oneDay:       return 86_400      // 1d
        case .oneWeek:      return 604_800     // 1w
        case .oneMonth:     return 2_592_000   // 1M (30d)
        case .threeMonths:  return 86_400      // 1d (Binance fallback)
        case .oneYear:      return 604_800     // 1w (Binance fallback)
        }
    }

    /// Time span a Binance chart covers at this range, in days.
    ///
    /// ``dataPointLimit`` candles × ``binanceIntervalSeconds`` per candle gives
    /// the width of the x-axis the user actually sees. Non-Binance sources use
    /// this as the target span so every chart shows the same history for the
    /// selected timeframe.
    var effectiveSpanDays: Int {
        Int((TimeInterval(dataPointLimit) * binanceIntervalSeconds / 86_400).rounded(.up))
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
        let needed = dataPointLimit * 2

        let interval: String
        if span <= 1      { interval = "1h" }
        else if span <= 2 { interval = "1d" }
        else if span <= 8 { interval = "1w" }
        else if span <= 31{ interval = "1m" }
        else              { interval = "max" }

        let intervalMinutes: Double = interval == "max" ? 43_200   // ~30 days
                                       : interval == "1m" ? 43_200
                                       : interval == "1w" ? 10_080
                                       : interval == "1d" ? 1_440
                                       : 60                       // "1h"

        let fidelity = max(1, Int(intervalMinutes / Double(needed)))

        return (interval, fidelity)
    }

    /// Maximum days of history CoinGecko should fetch for this timeframe.
    ///
    /// Derived from ``effectiveSpanDays`` — CG has no sub-day candles on the
    /// public tier, so shorter ranges accept a 1-day window.
    var preferredMaxDays: Int {
        max(1, effectiveSpanDays)
    }

    /// Number of candles to fetch from the API.
    var dataPointLimit: Int {
        switch self {
        case .oneHour:      return 48     // 2 days of 1h candles
        case .oneDay:       return 60     // ~2 months of 1d candles
        case .oneWeek:      return 26     // ~6 months of 1w candles
        case .oneMonth:     return 12     // 1 year of 1M candles
        case .threeMonths:  return 90     // 3 months of 1d candles
        case .oneYear:      return 52     // 1 year of 1w candles
        }
    }

}
