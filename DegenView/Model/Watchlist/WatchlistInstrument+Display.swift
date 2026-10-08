import Foundation

extension WatchlistInstrument {
    /// The asset whose artwork `IconResolver` should look up, matching what a chart card uses.
    var iconBaseSymbol: String {
        switch instrument.source {
        case .binance, .coinbase:
            return MarketSymbol(ticker: instrument.symbol, source: instrument.source)?.base
                ?? instrument.symbol.uppercased()
        case .coingecko, .dexscreener:
            return label.components(separatedBy: "/").first?.uppercased() ?? label.uppercased()
        case .polymarket, .kalshi:
            return displayName ?? name
        case .alpaca:
            return instrument.symbol.uppercased()
        case .coinMarketCap:
            return "CMC"
        }
    }

    /// The line under the name: the provider ("Binance"). Where the short label says something the name does not
    /// (a stock's ticker, a prediction market's outcome) it comes first: "AAPL · Alpaca (IEX)".
    var subtitle: String {
        let source = instrument.source.displayName
        guard label.caseInsensitiveCompare(name) != .orderedSame, !label.isEmpty else { return source }
        return "\(label) · \(source)"
    }

    /// Prediction markets have no price to cross, so they offer no price alert.
    var supportsPriceAlert: Bool {
        !instrument.source.isPredictionMarket
    }

    /// The asset the price-alert editor needs, shaped like the one a chart card builds.
    var alertAsset: PortfolioAsset {
        PortfolioAsset(
            key: "\(instrument.source.rawValue):\(instrument.symbol)",
            symbol: iconBaseSymbol, name: name, source: instrument.source,
            quoteCurrency: PortfolioCurrency(
                quoteSymbol: MarketSymbol(ticker: instrument.apiSymbol, source: instrument.source)?.quote),
            metadata: ["apiSymbol": instrument.apiSymbol])
    }
}
