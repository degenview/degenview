import Foundation

/// A trading pair split into its two assets, for showing one `BASE/QUOTE` form everywhere.
///
/// Exchanges persist their own ids — Binance `BTCUSDT`, Coinbase `BTC-USD` — and those stay the
/// identity of a chart. This only turns them into something to read.
struct MarketSymbol: Equatable {
    let base: String
    let quote: String

    var display: String { "\(base)/\(quote)" }

    /// Quote assets Binance lists pairs against. Matched longest first: `BTCUSDT` has to split on
    /// `USDT`, not `USD`, and `BTCFDUSD` on `FDUSD`, not leave `BTCFD` as the base.
    private static let binanceQuotes: [String] = [
        "FDUSD", "USDT", "USDC", "BUSD", "TUSD", "USDP", "DAI", "EUR", "GBP", "TRY", "BRL", "JPY", "USD", "BTC", "ETH",
        "BNB",
    ].sorted { $0.count > $1.count }

    /// Nil when the id isn't a pair this knows how to split — callers fall back to the raw id.
    init?(ticker: String, source: DataSourceType) {
        let id = ticker.trimmingCharacters(in: .whitespaces).uppercased()
        switch source {
        case .coinbase:
            let parts = id.split(whereSeparator: { $0 == "-" || $0 == "/" }).map(String.init)
            guard parts.count == 2 else { return nil }
            self.init(base: parts[0], quote: parts[1])
        case .binance:
            guard let quote = Self.binanceQuotes.first(where: { id.hasSuffix($0) && id.count > $0.count }) else {
                return nil
            }
            self.init(base: String(id.dropLast(quote.count)), quote: quote)
        case .coingecko, .dexscreener, .alpaca, .polymarket, .kalshi, .coinMarketCap:
            return nil
        }
    }

    init(base: String, quote: String) {
        self.base = base
        self.quote = quote
    }

    /// CoinGecko prices every chart in US dollars (`vs_currency=usd`) — not USDT, whatever the
    /// Binance cards beside it say.
    static func coinGecko(symbol: String) -> MarketSymbol {
        MarketSymbol(base: symbol.uppercased(), quote: "USD")
    }
}
