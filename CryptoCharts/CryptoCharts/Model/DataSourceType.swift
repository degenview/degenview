import Foundation

enum DataSourceType: String, CaseIterable, Codable {
    case binance     = "Binance"
    case coingecko   = "CoinGecko"
    case dexscreener = "DEXScreener"
    case polymarket  = "Polymarket"

    /// Crypto price sources — the set the multi-source ticker search fans out to.
    /// Polymarket is excluded: prediction markets get their own search pane.
    static var cryptoSources: [DataSourceType] {
        allCases.filter { $0 != .polymarket }
    }

    var displayName: String { rawValue }

    /// What this source's prices mean. Polymarket quotes probabilities in 0…1;
    /// everything else quotes USD.
    var priceScale: PriceScale {
        self == .polymarket ? .probability : .currency
    }

    var icon: String {
        switch self {
        case .binance:     return "building.columns.fill"
        case .coingecko:   return "chart.line.uptrend.xyaxis"
        case .dexscreener: return "arrow.triangle.swap"
        case .polymarket:  return "chart.line.flattrend.xyaxis"
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

    /// Human-readable label shown on the card. Only set for sources whose `symbol`
    /// is an opaque identifier — a Polymarket CLOB token id is 77 digits, so the
    /// market question has to ride along. Nil for crypto (the symbol reads fine).
    var displayName: String?
}
