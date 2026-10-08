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

    /// Parent event title for prediction markets (e.g. "How many Fed rate cuts in 2026?")
    var eventTitle: String? { metadata["eventTitle"]?.nilIfEmpty }
    /// Full market question for prediction markets
    var question: String? { metadata["question"]?.nilIfEmpty }
    /// Artwork supplied by the source itself, when the search payload carries one
    var imageURL: URL? { metadata["imageURL"]?.nilIfEmpty.flatMap(URL.init(string:)) }

    /// All tradable choices for multi-outcome prediction-market events. Nil for single-choice
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
    /// The quotes a bare ticker most likely means: Binance's dollar market, and Coinbase's.
    /// USDC and the rest stay behind them, in the provider's own order.
    static let primaryQuotes: Set<String> = ["USDT", "USD"]

    func searchRank(for needle: String) -> Int {
        let display = symbol.uppercased()
        let full = fullSymbol.uppercased()
        let pair = display.split(separator: "/", maxSplits: 1).map(String.init)
        let base = pair.first ?? display.components(separatedBy: " — ").first ?? display
        let quote = pair.count == 2 ? pair[1] : nil

        if display == needle || full == needle || base == needle && quote == nil { return 0 }
        if base == needle && quote.map(Self.primaryQuotes.contains) == true { return 1 }
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

/// Runs `fetch(0)…fetch(count - 1)` with at most `maxConcurrent` in flight and returns the results
/// in index order. The first error cancels what is still running and is rethrown. Replay history
/// is paged by time window, so its pages do not depend on each other and need not run one by one.
func fetchPagesConcurrently<T: Sendable>(
    count: Int,
    maxConcurrent: Int = 5,
    fetch: @escaping @Sendable (Int) async throws -> T
) async throws -> [T] {
    guard count > 0 else { return [] }
    return try await withThrowingTaskGroup(of: (Int, T).self) { group in
        var results = [T?](repeating: nil, count: count)
        var next = 0
        func addNext() {
            let index = next
            next += 1
            group.addTask { (index, try await fetch(index)) }
        }
        while next < min(count, maxConcurrent) { addNext() }
        while let (index, value) = try await group.next() {
            results[index] = value
            if next < count { addNext() }
        }
        return results.compactMap { $0 }
    }
}

/// One asset to price: the source's own symbol, plus whatever the source needs beyond it
/// (a DEX pair also needs its chain).
struct QuoteRequest: Hashable, Sendable {
    let symbol: String
    let metadata: [String: String]
}

/// What a source's 24h volume figure is measured in.
enum SourceVolumeKind: Sendable {
    /// Turnover in dollars (or a dollar stablecoin).
    case quoteCurrency
    case shares
    /// Units of the traded asset.
    case base
}

/// A current price with the price 24 hours earlier, when the source reports one.
///
/// Volume and the source's own timestamp ride along where the same call already returns them,
/// so a watchlist row costs no extra request.
struct SourceQuote: Equatable, Sendable {
    let price: Double
    let previousDayPrice: Double?
    var volume24h: Double? = nil
    var volumeKind: SourceVolumeKind? = nil
    /// When the source says this price was struck; nil when it does not say.
    var timestamp: Date? = nil
}

/// Optional capability for sources that can price many assets in one call, far cheaper than
/// reading the last close out of a candle request per asset. Results are keyed by
/// `QuoteRequest.symbol` exactly as passed in; a symbol the source didn't answer is omitted,
/// and the caller falls back to candles for it.
protocol BatchQuoteDataSource: TickerDataSource {
    func fetchQuotes(_ requests: [QuoteRequest]) async throws -> [String: SourceQuote]
}

extension SourceQuote {
    /// Sources that report a 24h percent change rather than a price: `previous = price / (1 + pct/100)`.
    init(price: Double, changePercent24h: Double?) {
        self.price = price
        if let changePercent24h, changePercent24h > -100 {
            previousDayPrice = price / (1 + changePercent24h / 100)
        } else {
            previousDayPrice = nil
        }
    }
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
    private lazy var coinbaseService = CoinbaseAPIService()
    private lazy var coinGeckoService = CoinGeckoAPIService()
    private lazy var dexScreenerService = DEXScreenerService()
    private lazy var alpacaService = AlpacaAPIService()
    private lazy var polymarketService = PolymarketService()
    private lazy var kalshiService = KalshiService()
    private lazy var coinMarketCapService = CoinMarketCapTickerDataSource()

    func service(for type: DataSourceType) -> TickerDataSource {
        switch type {
        case .binance: return binanceService
        case .coinbase: return coinbaseService
        case .coingecko: return coinGeckoService
        case .dexscreener: return dexScreenerService
        case .alpaca: return alpacaService
        case .polymarket: return polymarketService
        case .kalshi: return kalshiService
        case .coinMarketCap: return coinMarketCapService
        }
    }

    /// Crypto sources fanned out to by the multi-source ticker search, in priority order:
    /// the search lists results — and Enter picks the first hit — in this sequence.
    /// Prediction markets are searched separately — different query shape, different rows.
    var allSources: [TickerDataSource] {
        [binanceService, coinbaseService, coinGeckoService, dexScreenerService]
    }

    /// Concretely typed accessor — the Polymarket search pane and the chart fetch
    /// path both need `PolymarketService`'s non-protocol methods.
    var polymarket: PolymarketService { polymarketService }
    var kalshi: KalshiService { kalshiService }

    /// Provider behind a prediction-market source.
    func predictionMarket(for type: DataSourceType) -> PredictionMarketDataSource? {
        service(for: type) as? PredictionMarketDataSource
    }
    var alpaca: AlpacaAPIService { alpacaService }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
