import Foundation

struct KlineData: Identifiable, Codable {
    let id = UUID()
    let openTime: Date
    let openPrice: Double
    var highPrice: Double
    var lowPrice: Double
    var closePrice: Double
    var volume: Double

    /// `id` is excluded — it's a fresh per-instance identity, not persisted state.
    private enum CodingKeys: String, CodingKey {
        case openTime, openPrice, highPrice, lowPrice, closePrice, volume
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

        if let vol = Self.extractDouble(raw[5]) {
            self.volume = vol
        } else {
            self.volume = 0
        }
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
        self.volume = 0   // CoinGecko OHLC doesn't include volume
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
