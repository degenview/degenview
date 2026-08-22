import Foundation

// MARK: - Search Result

struct TickerSearchResult: Identifiable, Hashable {
    let id = UUID()
    /// Human-readable label, e.g. "BTC/USDT"
    let symbol: String
    /// API-specific identifier used for kline fetching
    let fullSymbol: String
    let source: DataSourceType
    let price: Double?

    /// Source-specific metadata (chain, dex name, pair address, etc.)
    var metadata: [String: String] = [:]

    /// Chain name for DEX pairs (e.g. "ethereum", "solana")
    var chain: String? { metadata["chain"] }
    /// DEX name (e.g. "uniswap", "raydium")
    var dex: String? { metadata["dex"] }
    /// Pair contract address for DEX pairs
    var pairAddress: String? { metadata["pairAddress"] }

    /// Parent event title for Polymarket markets (e.g. "How many Fed rate cuts in 2026?")
    var eventTitle: String? { metadata["eventTitle"]?.nilIfEmpty }
    /// Full market question for Polymarket markets
    var question: String? { metadata["question"]?.nilIfEmpty }
    /// Artwork supplied by the source itself, when the search payload carries one
    var imageURL: URL? { metadata["imageURL"]?.nilIfEmpty.flatMap(URL.init(string:)) }

    /// All tradable choices for multi-outcome Polymarket events. Nil for single-choice
    /// markets. When set, selecting this result adds all choices as separate chart lines.
    var pmSeries: [PmSeriesConfig]? = nil

    func hash(into hasher: inout Hasher) {
        hasher.combine(fullSymbol)
        hasher.combine(source)
    }

    static func == (lhs: TickerSearchResult, rhs: TickerSearchResult) -> Bool {
        lhs.fullSymbol == rhs.fullSymbol && lhs.source == rhs.source
    }
}

// MARK: - Protocol

protocol TickerDataSource: AnyObject {
    var type: DataSourceType { get }

    /// Fetch candlestick / OHLC data.
    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData]

    /// Synchronously return cached klines without hitting the network.
    /// Returns nil if no cache or cache miss. Used for instant-first-render.
    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]?

    /// Search for tickers matching a query string.
    func searchTickers(query: String) async throws -> [TickerSearchResult]
}

extension TickerDataSource {
    /// Default: no cache access. Services with caches override.
    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        return nil
    }
}

// MARK: - Factory

final class DataSourceFactory {
    static let shared = DataSourceFactory()

    private lazy var binanceService = BinanceAPIService()
    private lazy var coinGeckoService = CoinGeckoAPIService()
    private lazy var dexScreenerService = DEXScreenerService()
    private lazy var polymarketService = PolymarketService()

    func service(for type: DataSourceType) -> TickerDataSource {
        switch type {
        case .binance:     return binanceService
        case .coingecko:   return coinGeckoService
        case .dexscreener: return dexScreenerService
        case .polymarket:  return polymarketService
        }
    }

    /// Crypto sources fanned out to by the multi-source ticker search.
    /// Polymarket is searched separately — different query shape, different rows.
    var allSources: [TickerDataSource] {
        [binanceService, coinGeckoService, dexScreenerService]
    }

    /// Concretely typed accessor — the Polymarket search pane and the chart fetch
    /// path both need `PolymarketService`'s non-protocol methods.
    var polymarket: PolymarketService { polymarketService }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
