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
