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

    /// Search for tickers matching a query string.
    func searchTickers(query: String) async throws -> [TickerSearchResult]
}

// MARK: - Factory

final class DataSourceFactory {
    static let shared = DataSourceFactory()

    private lazy var binanceService = BinanceAPIService()
    private lazy var coinGeckoService = CoinGeckoAPIService()
    private lazy var dexScreenerService = DEXScreenerService()

    func service(for type: DataSourceType) -> TickerDataSource {
        switch type {
        case .binance:     return binanceService
        case .coingecko:   return coinGeckoService
        case .dexscreener: return dexScreenerService
        }
    }

    /// All sources available for searching.
    var allSources: [TickerDataSource] {
        [binanceService, coinGeckoService, dexScreenerService]
    }
}
