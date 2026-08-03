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

    /// Whether this source reports per-candle turnover for the volume bars to draw.
    ///
    /// Binance sends quote volume with every kline, and DEX pairs get theirs from
    /// GeckoTerminal. CoinGecko's OHLC endpoint has no volume column — only
    /// `/market_chart`, which reports a rolling 24h figure rather than per-candle —
    /// and Polymarket reports none at all.
    var providesVolume: Bool {
        switch self {
        case .binance, .dexscreener:  return true
        case .coingecko, .polymarket: return false
        }
    }

    /// Whether a larger `limit` buys *older* candles at the same interval.
    ///
    /// True for the count-based APIs. CoinGecko instead picks a days-window whose
    /// granularity follows the span, and Polymarket's count only sets downsample
    /// fidelity inside a fixed window — asking either for more would change the
    /// candle size or shrink the visible span rather than extend history, so
    /// indicator warm-up is fetched only where it actually works.
    var fetchesByCount: Bool {
        switch self {
        case .binance, .dexscreener:  return true
        case .coingecko, .polymarket: return false
        }
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

    /// Vertical price-scale zoom set by dragging the Y axis. nil = auto-fit (1.0).
    var yZoom: Double?

    /// Volume bars under the candles. nil = off.
    var showVolume: Bool?

    /// RSI line across the bottom of the plot. nil = off.
    var showRSI: Bool?

    /// EMA overlay on the price scale, and its period. nil = off / default period.
    var showEMA: Bool?
    var emaPeriod: Int?

    /// Bollinger bands on the price scale. nil = off.
    var showBollinger: Bool?

    /// Human-readable label shown on the card. Only set for sources whose `symbol`
    /// is an opaque identifier — a Polymarket CLOB token id is 77 digits, so the
    /// market question has to ride along. Nil for crypto (the symbol reads fine).
    var displayName: String?
}
