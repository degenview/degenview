import Foundation

enum DataSourceType: String, CaseIterable, Codable {
    case binance     = "Binance"
    case coingecko   = "CoinGecko"
    case dexscreener = "DEXScreener"

    var displayName: String { rawValue }

    var icon: String {
        switch self {
        case .binance:     return "building.columns.fill"
        case .coingecko:   return "chart.line.uptrend.xyaxis"
        case .dexscreener: return "arrow.triangle.swap"
        }
    }
}

/// Persisted config for a single ticker — symbol + which API to fetch from.
struct TickerConfig: Codable, Equatable, Hashable {
    let symbol: String
    let source: DataSourceType

    // Chart appearance settings (nil = use defaults)
    var bullishColorHex: String?
    var bearishColorHex: String?
    var yAxisDecimalPlaces: Int?  // nil = auto-detect
}
