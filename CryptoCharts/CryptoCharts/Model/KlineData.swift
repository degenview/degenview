import Foundation

struct KlineData: Identifiable, Codable {
    let id = UUID()
    let openTime: Date
    let openPrice: Double
    var highPrice: Double
    var lowPrice: Double
    var closePrice: Double
    var volume: Double
    /// Turnover in the quote currency (USD for a *USDT pair) over the candle.
    /// `volume` counts the base asset, so this is the one that plots as money.
    /// Zero for sources that don't report it — see the `init`s below.
    var quoteVolume: Double = 0

    /// `id` is excluded — it's a fresh per-instance identity, not persisted state.
    private enum CodingKeys: String, CodingKey {
        case openTime, openPrice, highPrice, lowPrice, closePrice, volume, quoteVolume
    }

    /// Hand-written so `quoteVolume` can be optional on the way in: it was added
    /// after the kline cache shipped, and a synthesized decoder would reject every
    /// entry already on disk. Encoding stays synthesized.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        openTime = try container.decode(Date.self, forKey: .openTime)
        openPrice = try container.decode(Double.self, forKey: .openPrice)
        highPrice = try container.decode(Double.self, forKey: .highPrice)
        lowPrice = try container.decode(Double.self, forKey: .lowPrice)
        closePrice = try container.decode(Double.self, forKey: .closePrice)
        volume = try container.decode(Double.self, forKey: .volume)
        quoteVolume = try container.decodeIfPresent(Double.self, forKey: .quoteVolume) ?? 0
    }

    init(
        openTime: Date,
        openPrice: Double,
        highPrice: Double,
        lowPrice: Double,
        closePrice: Double,
        volume: Double,
        quoteVolume: Double = 0
    ) {
        self.openTime = openTime
        self.openPrice = openPrice
        self.highPrice = highPrice
        self.lowPrice = lowPrice
        self.closePrice = closePrice
        self.volume = volume
        self.quoteVolume = quoteVolume
    }
}

extension KlineData {
    /// Parse from raw Binance kline array.
    /// Array layout: [openTime, open, high, low, close, volume, closeTime, quoteVolume, trades, takerBuyBase, takerBuyQuote, ignore]
    init?(raw: [Any]) {
        guard raw.count >= 6 else { return nil }

        guard let ts = Self.extractTimestamp(raw[0]) else { return nil }
        self.openTime = Date(timeIntervalSince1970: ts / 1000.0)

        guard let open = Self.extractDouble(raw[1]),
              let high = Self.extractDouble(raw[2]),
              let low  = Self.extractDouble(raw[3]),
              let close = Self.extractDouble(raw[4])
        else { return nil }

        self.openPrice = open
        self.highPrice = high
        self.lowPrice = low
        self.closePrice = close

        self.volume = Self.extractDouble(raw[5]) ?? 0
        // Index 7 is quote asset volume — turnover in USDT rather than in coins.
        self.quoteVolume = raw.count > 7 ? (Self.extractDouble(raw[7]) ?? 0) : 0
    }

    /// Parse from CoinGecko OHLC array.
    /// Array layout: [timestamp_ms, open, high, low, close]
    /// - Values are Double (or NSNumber) — timestamp is epoch ms, prices are USD.
    init?(rawCoinGecko: [Any]) {
        guard rawCoinGecko.count >= 5 else { return nil }

        guard let ts = Self.extractTimestamp(rawCoinGecko[0]) else { return nil }
        self.openTime = Date(timeIntervalSince1970: ts / 1000.0)

        guard let open = Self.extractDouble(rawCoinGecko[1]),
              let high = Self.extractDouble(rawCoinGecko[2]),
              let low  = Self.extractDouble(rawCoinGecko[3]),
              let close = Self.extractDouble(rawCoinGecko[4])
        else { return nil }

        self.openPrice = open
        self.highPrice = high
        self.lowPrice = low
        self.closePrice = close
        // CoinGecko's OHLC endpoint carries no volume; /market_chart would.
        self.volume = 0
        self.quoteVolume = 0
    }

    /// A single price observation, with no OHLC spread.
    ///
    /// Line-chart sources (Polymarket) report one price per timestamp. Flattening it
    /// into a candle keeps them on the shared pipeline — `KlineCache`, `currentPrice`,
    /// `priceChangePercent` and `ChartViewModel.fetchData` all work unchanged, and
    /// `LineChartView` only ever reads `openTime` and `closePrice`.
    init(time: Date, price: Double) {
        self.openTime = time
        self.openPrice = price
        self.highPrice = price
        self.lowPrice = price
        self.closePrice = price
        self.volume = 0
        self.quoteVolume = 0
    }

    // MARK: - Type-flexible parsers

    /// Extract a timestamp in milliseconds from Int64, Double, or NSNumber.
    private static func extractTimestamp(_ value: Any) -> Double? {
        if let v = value as? Int64 { return Double(v) }
        if let v = value as? Double { return v }
        if let v = value as? NSNumber { return v.doubleValue }
        return nil
    }

    /// Extract a Double from String, Double, or NSNumber.
    private static func extractDouble(_ value: Any) -> Double? {
        if let v = value as? String { return Double(v) }
        if let v = value as? Double { return v }
        if let v = value as? NSNumber { return v.doubleValue }
        return nil
    }
}

extension Array where Element == KlineData {
    /// Percentage change from first to last close price. Nil if fewer than 2 points.
    var priceChangePercent: Double? {
        guard let first = first, let last = last, first.closePrice != 0 else { return nil }
        return ((last.closePrice - first.closePrice) / first.closePrice) * 100
    }

    /// Merge into at most `count` candles by folding contiguous runs together.
    ///
    /// Unlike ``downsampled(to:)``, which throws points away, this keeps the extremes:
    /// open comes from the first candle in the run, close from the last, high/low from
    /// the whole run, and both volumes are summed. Sources with no weekly or monthly
    /// bucket (GeckoTerminal tops out at daily) use this to build one.
    func aggregated(into count: Int) -> [KlineData] {
        guard count > 0, self.count > count else { return self }

        var merged: [KlineData] = []
        merged.reserveCapacity(count)

        for bucket in 0..<count {
            let start = self.count * bucket / count
            let end = self.count * (bucket + 1) / count
            guard start < end else { continue }
            let run = self[start..<end]
            guard let first = run.first, let last = run.last else { continue }

            merged.append(
                KlineData(
                    openTime: first.openTime,
                    openPrice: first.openPrice,
                    highPrice: run.map(\.highPrice).max() ?? first.highPrice,
                    lowPrice: run.map(\.lowPrice).min() ?? first.lowPrice,
                    closePrice: last.closePrice,
                    volume: run.reduce(0) { $0 + $1.volume },
                    quoteVolume: run.reduce(0) { $0 + $1.quoteVolume }
                )
            )
        }
        return merged
    }

    /// Thin to at most `count` points by uniform stride, always keeping the newest one.
    ///
    /// APIs that only expose coarse bucket sizes (Polymarket's `fidelity`) hand back
    /// far more points than the zoom level asks for; this trims them without moving
    /// the window.
    func downsampled(to count: Int) -> [KlineData] {
        guard count > 0, self.count > count else { return self }
        guard count > 1 else { return last.map { [$0] } ?? [] }

        let stride = Double(self.count - 1) / Double(count - 1)
        var thinned: [KlineData] = []
        thinned.reserveCapacity(count)

        for i in 0..<count {
            let index = Int((Double(i) * stride).rounded())
            thinned.append(self[Swift.min(index, self.count - 1)])
        }
        return thinned
    }
}
