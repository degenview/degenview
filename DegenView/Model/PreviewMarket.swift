import Foundation

/// A market the Script Manager preview can chart. Crypto and stock sources only.
struct PreviewMarket: Codable, Equatable, Hashable, Identifiable {
    /// The ticker as `ChartViewModel` takes it (a search result's `fullSymbol`).
    var ticker: String
    var source: DataSourceType

    var id: String { "\(source.rawValue):\(ticker)" }

    static let `default` = PreviewMarket(ticker: "BTC", source: .binance)

    /// Whether the preview can chart `source`: exchanges, aggregators and DEX pairs, and stocks —
    /// not prediction markets, which draw lines no script runs on, or the CoinMarketCap indices.
    static func isSupported(_ source: DataSourceType) -> Bool {
        DataSourceType.cryptoSources.contains(source) || source == .alpaca
    }
}
