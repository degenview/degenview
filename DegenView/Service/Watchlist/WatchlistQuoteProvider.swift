import Foundation

/// Why a batch of quotes did not come back.
enum WatchlistQuoteFailure: Error, Equatable, Sendable {
    case rateLimited
    case credentialsMissing
    /// The provider refused the request itself, as it does for an unknown symbol. Retrying a smaller
    /// batch can find which symbol it was.
    case rejected(String)
    /// Network trouble or a server error; the whole source is treated as down for a moment.
    case failed(String)

    var message: String {
        switch self {
        case .rateLimited: return "Rate limited by the provider"
        case .credentialsMissing: return "Add your Alpaca API key in Settings to see stock quotes"
        case .rejected(let message), .failed(let message): return message
        }
    }

    static func classify(_ error: Error) -> WatchlistQuoteFailure {
        if let failure = error as? WatchlistQuoteFailure { return failure }
        switch error {
        case BinanceAPIError.rateLimited, CoinbaseAPIError.rateLimited, CoinGeckoError.rateLimited:
            return .rateLimited
        case AlpacaError.credentialsMissing:
            return .credentialsMissing
        case BinanceAPIError.symbolNotFound, CoinbaseAPIError.symbolNotFound, CoinGeckoError.coinNotFound:
            return .rejected(error.localizedDescription)
        case BinanceAPIError.httpError(let code) where (400..<500).contains(code):
            return .rejected(error.localizedDescription)
        default:
            return .failed(error.localizedDescription)
        }
    }
}

/// Where watchlist prices come from. The live one wraps the app's providers; tests substitute a recorder.
protocol WatchlistQuoteProvider: Sendable {
    /// Quotes keyed by `QuoteRequest.symbol`; a symbol the provider did not answer is omitted.
    func quotes(source: DataSourceType, requests: [QuoteRequest]) async throws -> [String: SourceQuote]
    /// The network of a DEX pair saved without one, from its address.
    func resolveChain(address: String) async -> String?
}

struct LiveWatchlistQuoteProvider: WatchlistQuoteProvider {
    let service: @Sendable (DataSourceType) -> any TickerDataSource

    static let shared = LiveWatchlistQuoteProvider { DataSourceFactory.shared.service(for: $0) }

    /// Prediction markets are read a few at a time; they have no batch call.
    private static let predictionConcurrency = 3

    func quotes(source: DataSourceType, requests: [QuoteRequest]) async throws -> [String: SourceQuote] {
        let provider = service(source)
        if let batch = provider as? BatchQuoteDataSource {
            return try await batch.fetchQuotes(requests)
        }
        if let prediction = provider as? PredictionMarketDataSource {
            return await Self.predictionQuotes(prediction, requests: requests)
        }
        return [:]
    }

    func resolveChain(address: String) async -> String? {
        guard let results = try? await service(.dexscreener).searchTickers(query: address) else { return nil }
        return results.first { $0.fullSymbol.caseInsensitiveCompare(address) == .orderedSame }?.chain
    }

    /// Last price of the past day's series, and its first price as the comparison.
    private static func predictionQuotes(
        _ provider: any PredictionMarketDataSource, requests: [QuoteRequest]
    ) async -> [String: SourceQuote] {
        var quotes: [String: SourceQuote] = [:]
        for start in stride(from: 0, to: requests.count, by: predictionConcurrency) {
            let page = requests[start..<min(start + predictionConcurrency, requests.count)]
            await withTaskGroup(of: (String, SourceQuote?).self) { group in
                for request in page {
                    group.addTask {
                        guard let series = try? await provider.fetchPrices(marketID: request.symbol, range: .oneDay, count: 30),
                            let last = series.last
                        else { return (request.symbol, nil) }
                        return (
                            request.symbol,
                            SourceQuote(price: last.closePrice, previousDayPrice: series.first?.closePrice))
                    }
                }
                for await (symbol, quote) in group { if let quote { quotes[symbol] = quote } }
            }
        }
        return quotes
    }
}
