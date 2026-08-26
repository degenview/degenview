import Foundation

actor MarketQuoteCoordinator {
    static let shared = MarketQuoteCoordinator()
    private var owners: [String: [String: PortfolioAsset]] = [:]
    private var latest: [String: MarketQuote] = [:]
    private var pollingTask: Task<Void, Never>?
    var onQuote: (@Sendable (MarketQuote) async -> Void)?

    func setQuoteHandler(_ handler: @escaping @Sendable (MarketQuote) async -> Void) { onQuote = handler }

    func subscribe(owner: String, assets: [PortfolioAsset]) {
        let assetsByKey = Self.assetsByKey(assets)
        guard !assetsByKey.isEmpty else {
            unsubscribe(owner: owner)
            return
        }
        owners[owner] = assetsByKey
        ensurePolling()
    }
    func unsubscribe(owner: String) {
        owners.removeValue(forKey: owner)
        if owners.isEmpty {
            pollingTask?.cancel()
            pollingTask = nil
        }
    }
    func latestQuote(for key: String) -> MarketQuote? { latest[key] }

    func ingest(_ quote: MarketQuote) async {
        guard latest[quote.asset.key]?.fingerprint != quote.fingerprint else { return }
        if let old = latest[quote.asset.key], quote.sourceTimestamp < old.sourceTimestamp { return }
        latest[quote.asset.key] = quote
        await onQuote?(quote)
    }

    private func ensurePolling() {
        guard pollingTask == nil else { return }
        pollingTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.poll()
                try? await Task.sleep(for: .seconds(15))
            }
        }
    }

    private func poll() async {
        let assets = Self.assetsByKey(owners.values.flatMap(\.values)).values
        let values = Array(assets.filter { $0.source != .polymarket })
        for chunkStart in stride(from: 0, to: values.count, by: 4) {
            let chunk = values[chunkStart..<min(chunkStart + 4, values.count)]
            await withTaskGroup(of: MarketQuote?.self) { group in
                for asset in chunk {
                    group.addTask {
                        do {
                            let symbol =
                                asset.metadata["apiSymbol"]
                                ?? String(asset.key.dropFirst(asset.source.rawValue.count + 1))
                            let interval = asset.source == .alpaca ? "1h" : "1m"
                            let candles = try await DataSourceFactory.shared.service(for: asset.source).fetchKlines(
                                symbol: symbol, interval: interval, limit: 2)
                            guard let candle = candles.last else { return nil }
                            let received = Date()
                            let sourceDate = candle.openTime
                            let quote = MarketQuote(
                                asset: asset, price: Decimal(candle.closePrice), currency: asset.quoteCurrency,
                                sourceTimestamp: sourceDate, receivedAt: received,
                                maximumAge: Self.maximumAge(for: asset.source),
                                fingerprint: "\(asset.key):\(sourceDate.timeIntervalSince1970):\(candle.closePrice)")
                            return quote
                        } catch { return nil }
                    }
                }
                for await quote in group { if let quote { await ingest(quote) } }
            }
        }
    }

    /// Quote polling needs one descriptor per logical asset. Alerts and portfolios can
    /// legitimately contain the same asset more than once, so duplicates are merged
    /// instead of using `Dictionary(uniqueKeysWithValues:)`, which traps.
    nonisolated static func assetsByKey<S: Sequence>(_ assets: S) -> [String: PortfolioAsset]
    where S.Element == PortfolioAsset {
        assets.reduce(into: [:]) { result, asset in
            if result[asset.key] == nil { result[asset.key] = asset }
        }
    }

    private static func maximumAge(for source: DataSourceType) -> TimeInterval {
        switch source {
        case .binance: 180
        case .alpaca: 7_200
        case .coingecko, .dexscreener: 1_800
        case .polymarket: 0
        }
    }
}
