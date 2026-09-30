import Foundation

/// A source of binary-outcome markets — YES probabilities in 0…1, charted as lines.
///
/// `ChartViewModel` talks to Polymarket and Kalshi through this. It takes the whole
/// `TimeRange` rather than the protocol's interval token because that token is lossy:
/// it maps 1D and 3M both onto `"1d"`.
protocol PredictionMarketDataSource: TickerDataSource {
    /// The YES price series for a market. `marketID` is the provider's opaque id — a
    /// CLOB token id on Polymarket, `"SERIES/MARKET"` on Kalshi.
    func fetchPrices(marketID: String, range: TimeRange, count: Int) async throws -> [KlineData]
}

enum PredictionMarketPrice {
    /// Overlay the executable YES ask on the final history sample so the chart's
    /// endpoint, current-price line, and headline match the provider's market page.
    static func replacingLastPrice(in data: [KlineData], with price: Double) -> [KlineData] {
        guard !data.isEmpty, price.isFinite, (0...1).contains(price) else { return data }
        var result = data
        result[result.count - 1].closePrice = price
        result[result.count - 1].highPrice = price
        result[result.count - 1].lowPrice = price
        return result
    }
}
