import Foundation

enum DataSourceType: String, CaseIterable, Codable {
    case binance     = "Binance"
    case coingecko   = "CoinGecko"
    case dexscreener = "DEXScreener"
    case alpaca      = "Alpaca (IEX)"
    case polymarket  = "Polymarket"

    /// Crypto price sources — the set the multi-source ticker search fans out to.
    /// Polymarket is excluded: prediction markets get their own search pane.
    static var cryptoSources: [DataSourceType] {
        [.binance, .coingecko, .dexscreener]
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
        case .binance, .dexscreener, .alpaca: return true
        case .coingecko, .polymarket:         return false
        }
    }

    /// Whether a larger `limit` buys *older* candles at the same interval.
    ///
    /// True everywhere except Polymarket, whose count only sets downsample fidelity
    /// inside a fixed window — asking for more shrinks the step rather than extending
    /// history, so indicator warm-up is fetched only where it actually works.
    ///
    /// CoinGecko qualifies because its window is a fixed span per interval, deep
    /// enough for the furthest zoom-out: a larger `limit` slices further back into a
    /// buffer already fetched, at the same candle size, without another request.
    var fetchesByCount: Bool {
        switch self {
        case .binance, .dexscreener, .coingecko, .alpaca: return true
        case .polymarket:                        return false
        }
    }

    var icon: String {
        switch self {
        case .binance:     return "building.columns.fill"
        case .coingecko:   return "chart.line.uptrend.xyaxis"
        case .dexscreener: return "arrow.triangle.swap"
        case .alpaca:      return "chart.xyaxis.line"
        case .polymarket:  return "chart.line.flattrend.xyaxis"
        }
    }
}

enum ChartAssetType: String, CaseIterable, Identifiable {
    case crypto = "Crypto"
    case stock = "Stock"
    case polymarket = "Polymarket"

    var id: String { rawValue }
}

/// One tradable choice within a Polymarket event — a token ID + human label pair.
struct PmSeriesConfig: Codable, Equatable, Hashable, Identifiable {
    let tokenID: String
    let label: String
    var enabled: Bool

    var id: String { tokenID }
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

    /// Legacy trend-line storage. New versions migrate this into DrawingStore and
    /// always write nil so drawings are not attached to tabs or saved views.
    var trendLines: [TrendLine]?

    /// Human-readable label shown on the card. Only set for sources whose `symbol`
    /// is an opaque identifier — a Polymarket CLOB token id is 77 digits, so the
    /// market question has to ride along. Nil for crypto (the symbol reads fine).
    var displayName: String?

    /// All tradable choices for multi-outcome Polymarket events. Nil for single-choice
    /// markets and all non-Polymarket sources.
    var pmSeries: [PmSeriesConfig]?
}
