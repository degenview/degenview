import Foundation

enum TimeRange: String, CaseIterable, Identifiable {
    case oneHour = "1H"
    case oneDay = "1D"
    case oneWeek = "1W"
    case oneMonth = "1M"
    case threeMonths = "3M"
    case oneYear = "1Y"

    var id: String { rawValue }

    /// Binance kline interval string.
    var binanceInterval: String {
        switch self {
        case .oneHour:      return "1m"
        case .oneDay:       return "15m"
        case .oneWeek:      return "1h"
        case .oneMonth:     return "4h"
        case .threeMonths:  return "12h"
        case .oneYear:      return "1d"
        }
    }

    /// Number of candles to fetch from the API.
    var dataPointLimit: Int {
        switch self {
        case .oneHour:      return 60
        case .oneDay:       return 96
        case .oneWeek:      return 168
        case .oneMonth:     return 180
        case .threeMonths:  return 180
        case .oneYear:      return 365
        }
    }

    var chartTitle: String {
        switch self {
        case .oneHour:      return "Last Hour"
        case .oneDay:       return "Last 24 Hours"
        case .oneWeek:      return "Last Week"
        case .oneMonth:     return "Last Month"
        case .threeMonths:  return "Last 3 Months"
        case .oneYear:      return "Last Year"
        }
    }

    /// DateFormatter style for the X axis depending on range.
    var dateFormat: String {
        switch self {
        case .oneHour:      return "HH:mm"
        case .oneDay:       return "HH:mm"
        case .oneWeek:      return "EEE HH:mm"
        case .oneMonth:     return "MMM d"
        case .threeMonths:  return "MMM d"
        case .oneYear:      return "MMM yy"
        }
    }
}
