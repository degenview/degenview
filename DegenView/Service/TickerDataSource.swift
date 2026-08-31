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

// MARK: - Search Relevance

extension Array where Element == TickerSearchResult {
    /// Stable, provider-local ordering that favors the asset a ticker query most
    /// likely refers to. Equal-ranked results retain the source's original order.
    func ranked(for query: String) -> [TickerSearchResult] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        guard !needle.isEmpty else { return self }

        return enumerated().sorted { lhs, rhs in
            let leftRank = lhs.element.searchRank(for: needle)
            let rightRank = rhs.element.searchRank(for: needle)
            return leftRank == rightRank ? lhs.offset < rhs.offset : leftRank < rightRank
        }.map(\.element)
    }
}

private extension TickerSearchResult {
    func searchRank(for needle: String) -> Int {
        let display = symbol.uppercased()
        let full = fullSymbol.uppercased()
        let pair = display.split(separator: "/", maxSplits: 1).map(String.init)
        let base = pair.first ?? display.components(separatedBy: " — ").first ?? display
        let quote = pair.count == 2 ? pair[1] : nil

        if display == needle || full == needle || base == needle && quote == nil { return 0 }
        if base == needle && quote == "USDT" { return 1 }
        if base == needle && quote != nil { return 2 }
        if display.hasPrefix(needle) || full.hasPrefix(needle) || base.hasPrefix(needle) { return 3 }
        if display.contains(needle) || full.contains(needle) { return 4 }
        return 5
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

/// Optional capability for sources that expose timestamp-bounded OHLCV at a
/// resolution finer than the displayed chart. Replay never assumes this exists.
protocol GranularReplayDataSource: TickerDataSource {
    func supportedReplayIntervals(chartInterval: String) -> [ReplayInterval]
    func fetchReplayKlines(
        symbol: String,
        interval: ReplayInterval,
        start: Date,
        end: Date,
        maximumCount: Int
    ) async throws -> [KlineData]
}

extension TickerDataSource {
    /// Default: no cache access. Services with caches override.
    func getCachedKlines(symbol: String, interval: String, count: Int) async -> [KlineData]? {
        return nil
    }
}

final class CoinMarketCapTickerDataSource: TickerDataSource {
    let type: DataSourceType = .coinMarketCap
    func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
    func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
}

// MARK: - Factory

final class DataSourceFactory {
    static let shared = DataSourceFactory()

    private lazy var binanceService = BinanceAPIService()
    private lazy var coinGeckoService = CoinGeckoAPIService()
    private lazy var dexScreenerService = DEXScreenerService()
    private lazy var alpacaService = AlpacaAPIService()
    private lazy var polymarketService = PolymarketService()
    private lazy var coinMarketCapService = CoinMarketCapTickerDataSource()

    func service(for type: DataSourceType) -> TickerDataSource {
        switch type {
        case .binance: return binanceService
        case .coingecko: return coinGeckoService
        case .dexscreener: return dexScreenerService
        case .alpaca: return alpacaService
        case .polymarket: return polymarketService
        case .coinMarketCap: return coinMarketCapService
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
    var alpaca: AlpacaAPIService { alpacaService }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
