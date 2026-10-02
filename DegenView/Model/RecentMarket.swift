import Foundation

/// A market the user added a chart (or favorite, or portfolio asset) for, kept so it can be
/// picked again from the Add Chart sheet. Stores what a search result needs to be selected
/// without searching again; the price is deliberately dropped, since it would be stale.
struct RecentMarket: Codable, Hashable, Identifiable {
    let symbol: String
    let fullSymbol: String
    let source: DataSourceType
    var metadata: [String: String] = [:]

    var id: String { "\(source.rawValue):\(fullSymbol)" }

    init(_ result: TickerSearchResult) {
        symbol = result.symbol
        fullSymbol = result.fullSymbol
        source = result.source
        metadata = result.metadata
    }

    var result: TickerSearchResult {
        TickerSearchResult(symbol: symbol, fullSymbol: fullSymbol, source: source, price: nil, metadata: metadata)
    }
}
