import Foundation

/// Current prices for portfolio assets, as fast as each source allows.
///
/// Assets are grouped by source and each group is priced with one batched call where the source
/// has one (`BatchQuoteDataSource`). Anything a batch doesn't answer — a source without a batch
/// call, a failed request, a symbol it didn't return — falls back to reading the last close out
/// of recent hourly candles, which is what every asset used to cost.
struct PortfolioQuoteFetcher: Sendable {
    let service: @Sendable (DataSourceType) -> any TickerDataSource

    static let live = PortfolioQuoteFetcher { DataSourceFactory.shared.service(for: $0) }

    /// Keyed by `PortfolioAsset.key`. Non-USD assets are skipped: their price isn't a dollar price.
    func quotes(for assets: [PortfolioAsset]) async -> [String: PortfolioQuote] {
        let dollarAssets = assets.filter { $0.quoteCurrency == .USD }
        let bySource = Dictionary(grouping: dollarAssets, by: \.source)
        return await withTaskGroup(of: [String: PortfolioQuote].self) { group in
            for (source, assets) in bySource {
                group.addTask { await quotes(from: source, assets: assets) }
            }
            var result: [String: PortfolioQuote] = [:]
            for await partial in group { result.merge(partial) { first, _ in first } }
            return result
        }
    }

    private func quotes(from source: DataSourceType, assets: [PortfolioAsset]) async -> [String: PortfolioQuote] {
        let service = service(source)
        var result: [String: PortfolioQuote] = [:]
        var remaining = assets
        if let batch = service as? BatchQuoteDataSource {
            let requests = assets.map { QuoteRequest(symbol: $0.marketSymbol, metadata: $0.metadata) }
            let answered = (try? await batch.fetchQuotes(requests)) ?? [:]
            let now = Date()
            for asset in assets {
                guard let quote = answered[asset.marketSymbol] else { continue }
                result[asset.key] = PortfolioQuote(
                    price: Decimal(quote.price), previousDayPrice: quote.previousDayPrice.map { Decimal($0) },
                    timestamp: now)
            }
            remaining = assets.filter { result[$0.key] == nil }
        }
        guard !remaining.isEmpty else { return result }
        let fallbacks = await withTaskGroup(of: (String, PortfolioQuote?).self) { group in
            for asset in remaining {
                group.addTask { (asset.key, await candleQuote(for: asset, service: service)) }
            }
            var found: [String: PortfolioQuote] = [:]
            for await (key, quote) in group { if let quote { found[key] = quote } }
            return found
        }
        return result.merging(fallbacks) { first, _ in first }
    }

    /// The last close, and the close about a day earlier, from the last 25 hourly candles.
    private func candleQuote(for asset: PortfolioAsset, service: any TickerDataSource) async -> PortfolioQuote? {
        guard let data = try? await service.fetchKlines(symbol: asset.marketSymbol, interval: "1h", limit: 25),
            let latest = data.last
        else { return nil }
        let prior = data.last(where: { latest.openTime.timeIntervalSince($0.openTime) >= 23 * 3600 })
        return PortfolioQuote(
            price: Decimal(latest.closePrice), previousDayPrice: prior.map { Decimal($0.closePrice) },
            timestamp: Date())
    }
}
