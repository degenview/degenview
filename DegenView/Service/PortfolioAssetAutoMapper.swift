import Foundation

enum PortfolioAssetAutoMapper {
    static func resolve(symbol: String, baseCurrency: PortfolioCurrency) async -> PortfolioAsset? {
        let token = symbol.uppercased()
        let preferredQuotes = quoteSymbols(for: baseCurrency)

        // Binance is intentionally first: an exact BASE/QUOTE market gives the
        // portfolio a direct, liquid valuation instrument such as FLOKI/USDT.
        let binance = DataSourceFactory.shared.service(for: .binance)
        for quote in preferredQuotes {
            let requestedPair = "\(token)\(quote)"
            if let results = try? await binance.searchTickers(query: requestedPair),
               let match = results.first(where: { $0.fullSymbol.caseInsensitiveCompare(requestedPair) == .orderedSame }) {
                return PortfolioAsset(searchResult: match)
            }
        }

        // CoinGecko is the next safest identity source because its fullSymbol is
        // a stable coin id rather than a ticker. DEX pairs are the final fallback.
        if let results = try? await DataSourceFactory.shared.service(for: .coingecko).searchTickers(query: token),
           let exact = results.first(where: { $0.symbol.caseInsensitiveCompare(token) == .orderedSame }) {
            return PortfolioAsset(searchResult: exact)
        }
        if let results = try? await DataSourceFactory.shared.service(for: .dexscreener).searchTickers(query: token),
           let match = bestPair(in: results, token: token, preferredQuotes: preferredQuotes)
                ?? results.first(where: { baseSymbol(of: $0.symbol) == token }) {
            return PortfolioAsset(searchResult: match)
        }
        return nil
    }

    static func bestPair(in results: [TickerSearchResult], token: String,
                         preferredQuotes: [String]) -> TickerSearchResult? {
        let exactBase = results.filter { baseSymbol(of: $0.symbol) == token.uppercased() }
        for quote in preferredQuotes {
            if let result = exactBase.first(where: { quoteSymbol(of: $0.symbol) == quote }) { return result }
        }
        return nil
    }

    private static func quoteSymbols(for currency: PortfolioCurrency) -> [String] {
        switch currency {
        case .USD: return ["USDT", "USDC", "USD"]
        default: return [currency.rawValue, "USDT", "USDC", "USD"]
        }
    }

    private static func baseSymbol(of pair: String) -> String {
        pair.uppercased().split(separator: "/").first.map(String.init) ?? pair.uppercased()
    }
    private static func quoteSymbol(of pair: String) -> String? {
        let parts = pair.uppercased().split(separator: "/")
        return parts.count > 1 ? String(parts[1]) : nil
    }
}
