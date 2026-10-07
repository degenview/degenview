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

    /// The cached quote while it is fresh; otherwise a one-off fetch. Polling only covers assets
    /// with an active alert, so a brand-new alert's asset has no cached quote. The fetched quote is
    /// not ingested — an unwatched asset must not feed the alert engine.
    func quote(for asset: PortfolioAsset) async -> MarketQuote? {
        if let cached = latest[asset.key], cached.isFresh { return cached }
        return await Self.fetchQuote(for: asset)
    }

    func ingest(_ quote: MarketQuote) async {
        if let old = latest[quote.asset.key] {
            if quote.sourceTimestamp < old.sourceTimestamp { return }
            // Same price and candle: nothing new to announce, but the read is newer — keep it fresh.
            if old.fingerprint == quote.fingerprint {
                latest[quote.asset.key] = quote
                return
            }
        }
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
        let values = Array(assets.filter { !$0.source.isPredictionMarket })
        for chunkStart in stride(from: 0, to: values.count, by: 4) {
            let chunk = values[chunkStart..<min(chunkStart + 4, values.count)]
            await withTaskGroup(of: MarketQuote?.self) { group in
                for asset in chunk {
                    group.addTask { await Self.fetchQuote(for: asset) }
                }
                for await quote in group { if let quote { await ingest(quote) } }
            }
        }
    }

    /// One REST read of the latest candle close, without touching the cache or the quote handler.
    nonisolated static func fetchQuote(for asset: PortfolioAsset) async -> MarketQuote? {
        guard !asset.source.isPredictionMarket else { return nil }
        do {
            let symbol =
                asset.metadata["apiSymbol"]
                ?? String(asset.key.dropFirst(asset.source.rawValue.count + 1))
            let interval = asset.source == .alpaca ? "1h" : "1m"
            let candles = try await DataSourceFactory.shared.service(for: asset.source).fetchKlines(
                symbol: symbol, interval: interval, limit: 2)
            guard let candle = candles.last else { return nil }
            let received = Date()
            // Coinbase emits no candle for a minute without trades, so a quiet pair's newest candle
            // can be many minutes old while its close is still the last price. The response itself
            // is current; age it from now rather than from the candle.
            let sourceDate = asset.source == .coinbase ? received : candle.openTime
            return MarketQuote(
                asset: asset, price: Decimal(candle.closePrice), currency: asset.quoteCurrency,
                sourceTimestamp: sourceDate, receivedAt: received,
                maximumAge: maximumAge(for: asset.source),
                fingerprint: "\(asset.key):\(candle.openTime.timeIntervalSince1970):\(candle.closePrice)",
                candle: AlertCandleSnapshot(
                    openTime: candle.openTime, open: candle.openPrice, high: candle.highPrice,
                    low: candle.lowPrice, close: candle.closePrice, volume: candle.volume, interval: interval))
        } catch { return nil }
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
        case .binance, .coinbase: 180
        case .alpaca: 7_200
        case .coingecko, .dexscreener: 1_800
        case .polymarket, .kalshi, .coinMarketCap: 0
        }
    }
}
