import Foundation

/// Where a `request.security` series comes from: which data source, which symbol, which candle size.
/// Resolves what a script wrote (`"BINANCE:ETHUSDT"`, `syminfo.tickerid`, a timeframe in seconds) onto what
/// the app can fetch; anything else is nil, and the script is told there is no data for it.
struct PineSecurityTarget: Equatable {
    let source: DataSourceType
    let symbol: String
    let range: TimeRange

    /// The chart's own symbol as the app fetches it.
    struct Chart: Equatable {
        let tickerID: String
        let source: DataSourceType
        let apiSymbol: String
    }

    /// Exchanges TradingView prefixes that the app has a source for.
    private static let exchanges: [String: DataSourceType] = [
        "BINANCE": .binance, "COINBASE": .coinbase,
        "NASDAQ": .alpaca, "NYSE": .alpaca, "AMEX": .alpaca, "ARCA": .alpaca, "BATS": .alpaca,
    ]
    private static let quotes = ["USDT", "USDC", "BUSD", "USD", "EUR", "GBP", "BTC", "ETH"]

    static func resolve(_ key: PineSecurityKey, chart: Chart) -> PineSecurityTarget? {
        guard let range = range(forSeconds: key.interval) else { return nil }
        if key.symbol == chart.tickerID {
            return PineSecurityTarget(source: chart.source, symbol: chart.apiSymbol, range: range)
        }
        let parts = key.symbol.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false).map(String.init)
        let source: DataSourceType
        let name: String
        if parts.count == 2 {
            guard let known = exchanges[parts[0].uppercased()] else { return nil }
            (source, name) = (known, parts[1])
        } else {
            (source, name) = (chart.source, key.symbol)
        }
        guard !name.isEmpty, let symbol = apiSymbol(name, on: source) else { return nil }
        return PineSecurityTarget(source: source, symbol: symbol, range: range)
    }

    /// The app's candle size closest to `seconds`, if it is within 2 % of one (a Pine "12M" is 360 days, the
    /// app's `1Y` is 365).
    static func range(forSeconds seconds: TimeInterval) -> TimeRange? {
        TimeRange.allCases.first { abs($0.binanceIntervalSeconds - seconds) <= $0.binanceIntervalSeconds * 0.02 }
    }

    /// `name` the way `source`'s API wants it. Nil for sources whose symbols are not names a script can spell.
    private static func apiSymbol(_ name: String, on source: DataSourceType) -> String? {
        let upper = name.uppercased()
        switch source {
        case .binance: return upper
        case .coinbase:
            if upper.contains("-") || upper.contains("/") { return CoinbaseAPIService.productID(upper) }
            for quote in quotes where upper.hasSuffix(quote) && upper.count > quote.count {
                return "\(upper.dropLast(quote.count))-\(quote)"
            }
            return CoinbaseAPIService.productID(upper)
        case .alpaca: return upper
        default: return nil
        }
    }
}
