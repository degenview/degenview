import Foundation

/// Recent hourly prices for many coins at once, used to draw provisional candles
/// while the real OHLC request waits its turn in the rate-limited queue.
///
/// CoinGecko's `/coins/markets` endpoint returns a 7-day hourly sparkline for up
/// to 250 coins in a single call. That's one request for the whole screen, where
/// per-chart OHLC costs one request each — so it's the only way to put candles in
/// front of the user before the queue drains.
///
/// The candles it builds are approximations: open/close come from the first and
/// last hourly sample in each bucket, so wicks are shallower than the real thing
/// and the window is capped at 7 days. Every chart replaces them with true OHLC
/// as soon as its own fetch lands.
actor CoinGeckoProvisionalStore {

    struct PricePoint {
        let time: Date
        let price: Double
    }

    private var series: [String: [PricePoint]] = [:]
    private var primedAt: Date = .distantPast
    private var primeTask: Task<Void, Never>?

    /// Run `load` once for all callers that arrive while it's in flight.
    /// Re-primes only after `ttl` has elapsed.
    func ensurePrimed(ttl: TimeInterval, load: @escaping () async -> [String: [PricePoint]]) async {
        if let primeTask {
            await primeTask.value
            return
        }
        guard Date().timeIntervalSince(primedAt) > ttl else { return }

        let task = Task {
            let loaded = await load()
            self.store(loaded)
        }
        primeTask = task
        await task.value
        primeTask = nil
    }

    private func store(_ loaded: [String: [PricePoint]]) {
        // Stamped even when the load came back empty: a failed prime must not
        // send every remaining chart off to retry it.
        primedAt = Date()
        guard !loaded.isEmpty else { return }
        series.merge(loaded) { _, new in new }
    }

    /// Latest price seen for a coin, if primed.
    func lastPrice(for coinID: String) -> Double? {
        series[coinID.lowercased()]?.last?.price
    }

    /// Build up to `limit` candles at roughly `granularity`, newest last.
    ///
    /// Returns fewer candles than asked for when 7 days of hourly samples can't
    /// fill the window — a short chart now beats a full chart in 20 seconds, and
    /// the real fetch extends it.
    func candles(for coinID: String, granularity: TimeInterval, limit: Int) -> [KlineData]? {
        let points = series[coinID.lowercased()] ?? []
        guard points.count >= 2, limit > 0 else { return nil }

        // Samples are hourly, so a bucket is however many hours the target
        // candle spans. Sub-hourly targets degenerate to one sample per candle.
        let hoursPerCandle = max(1, Int((granularity / 3_600).rounded()))

        var candles: [KlineData] = []
        var end = points.count

        while end > 0, candles.count < limit {
            let start = max(0, end - hoursPerCandle)
            let bucket = points[start..<end]
            guard let open = bucket.first, let close = bucket.last else { break }

            let prices = bucket.map(\.price)
            candles.append(
                KlineData(
                    openTime: open.time,
                    openPrice: open.price,
                    highPrice: prices.max() ?? open.price,
                    lowPrice: prices.min() ?? open.price,
                    closePrice: close.price,
                    volume: 0
                )
            )
            end = start
        }

        guard !candles.isEmpty else { return nil }
        return candles.reversed()
    }
}
