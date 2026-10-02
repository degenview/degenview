import Foundation

/// Markets offered to someone with nothing of their own to pick from yet, in Add Chart and on
/// an empty tab. Crypto is by base asset on Binance; stocks are Alpaca symbols.
enum MarketSuggestions {
    static let crypto = ["BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK"]
    static let stocks = ["AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "SPY", "QQQ", "AMD"]

    /// The Binance USDT market for a base asset, shaped like a search result for it.
    static func cryptoResult(_ base: String) -> TickerSearchResult {
        TickerSearchResult(symbol: "\(base)/USDT", fullSymbol: "\(base)USDT", source: .binance, price: nil)
    }

    static func stockResult(_ symbol: String) -> TickerSearchResult {
        TickerSearchResult(symbol: symbol, fullSymbol: symbol, source: .alpaca, price: nil)
    }
}
