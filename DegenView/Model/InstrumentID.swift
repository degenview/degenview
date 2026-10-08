import Foundation

/// A market on one provider. The symbol alone never identifies a market: `BTCUSDT` on
/// Binance and `BTC-USD` on Coinbase are different instruments, and a display title is
/// never identity.
///
/// Equality and hashing use `key` only. `chain` is DEX metadata the quote endpoints need
/// but a pair address already identifies the market without it.
struct InstrumentID: Codable, Hashable, Sendable {
    let source: DataSourceType
    /// The provider's own id: a Binance symbol, a Coinbase product id, a CoinGecko coin id,
    /// a DEX pair address, a Polymarket token id, a Kalshi `SERIES/MARKET`.
    let symbol: String
    /// DEX network (`solana`, `ethereum`). Nil for every other source, and for DEX pairs
    /// saved before the chain was kept.
    var chain: String?

    init(source: DataSourceType, symbol: String, chain: String? = nil) {
        self.source = source
        self.symbol = symbol.trimmingCharacters(in: .whitespacesAndNewlines)
        self.chain = chain.flatMap { $0.isEmpty ? nil : $0 }
    }

    init(searchResult: TickerSearchResult) {
        self.init(
            source: searchResult.source,
            symbol: searchResult.fullSymbol,
            chain: searchResult.chain)
    }

    /// Nil for the card kinds that are not markets: portfolio, CoinMarketCap and power-law
    /// slots carry sentinel symbols.
    init?(config: TickerConfig) {
        guard config.portfolioChart == nil, config.coinMarketCapChart == nil,
            config.bitcoinPowerLaw == nil, config.source.isWatchlistInstrumentSource
        else { return nil }
        self.init(source: config.source, symbol: config.symbol)
    }

    /// Source-qualified identity, stable across launches and case-insensitive where the
    /// provider is. DEX addresses and prediction-market ids are case-sensitive.
    var key: String {
        "\(source.rawValue):\(Self.normalized(symbol, source: source))"
    }

    /// `BINANCE:BTCUSDT`, the form used for copying and for import/export.
    var qualifiedSymbol: String {
        "\(source.watchlistSlug):\(symbol)"
    }

    /// The id the provider's API takes. A bare Binance asset (`BTC`) is quoted in USDT and
    /// Coinbase ids go through `CoinbaseAPIService.productID`.
    var apiSymbol: String {
        switch source {
        case .binance:
            let upper = symbol.uppercased()
            if upper.hasSuffix("USDT") || upper.hasSuffix("USDC") || upper.hasSuffix("BUSD") { return upper }
            return "\(upper)USDT"
        case .coinbase:
            return CoinbaseAPIService.productID(symbol)
        case .coingecko, .dexscreener, .alpaca, .polymarket, .kalshi, .coinMarketCap:
            return symbol
        }
    }

    static func normalized(_ symbol: String, source: DataSourceType) -> String {
        switch source {
        case .binance, .coinbase, .alpaca: return symbol.uppercased()
        case .coingecko: return symbol.lowercased()
        case .dexscreener, .polymarket, .kalshi, .coinMarketCap: return symbol
        }
    }

    static func == (lhs: InstrumentID, rhs: InstrumentID) -> Bool {
        lhs.key == rhs.key
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(key)
    }
}

extension DataSourceType {
    /// CoinMarketCap cards are index charts, not markets anyone can watch.
    var isWatchlistInstrumentSource: Bool {
        self != .coinMarketCap
    }

    /// Upper-case provider prefix in the import/export text format.
    var watchlistSlug: String {
        switch self {
        case .binance: return "BINANCE"
        case .coinbase: return "COINBASE"
        case .coingecko: return "COINGECKO"
        case .dexscreener: return "DEXSCREENER"
        case .alpaca: return "ALPACA"
        case .polymarket: return "POLYMARKET"
        case .kalshi: return "KALSHI"
        case .coinMarketCap: return "CMC"
        }
    }

    init?(watchlistSlug slug: String) {
        guard
            let match = Self.allCases.first(where: {
                $0.isWatchlistInstrumentSource && $0.watchlistSlug == slug.uppercased()
            })
        else { return nil }
        self = match
    }
}
