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
    /// True when a live provider explicitly identifies the bar's closing update.
    var isClosed: Bool = false

    /// `id` is excluded — it's a fresh per-instance identity, not persisted state.
    private enum CodingKeys: String, CodingKey {
        case openTime, openPrice, highPrice, lowPrice, closePrice, volume, quoteVolume, isClosed
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
        isClosed = try container.decodeIfPresent(Bool.self, forKey: .isClosed) ?? false
    }

    init(
        openTime: Date,
        openPrice: Double,
        highPrice: Double,
        lowPrice: Double,
        closePrice: Double,
        volume: Double,
        quoteVolume: Double = 0,
        isClosed: Bool = false
    ) {
        self.openTime = openTime
        self.openPrice = openPrice
        self.highPrice = highPrice
        self.lowPrice = lowPrice
        self.closePrice = closePrice
        self.volume = volume
        self.quoteVolume = quoteVolume
        self.isClosed = isClosed
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

    // MARK: - Candles from a price series

    /// Build candles of `interval` seconds from timestamped prices, newest last.
    ///
    /// Sources that report a price series rather than candles still have to draw as
    /// candles here — CoinGecko's `/market_chart` is the only endpoint of theirs that
    /// covers an arbitrary span at a fine enough step to give one candle per hour or
    /// per day, and its samples are plain prices.
    ///
    /// Buckets open where Binance opens its klines, so a card built from prices names
    /// the same periods as the Binance card beside it — see ``bucketStart(of:interval:)``.
    /// Each candle opens at the previous one's close, so the series is continuous rather
    /// than a row of gaps; only the first opens at its own first sample.
    ///
    /// How much of a candle this recovers depends on how many samples land inside it.
    /// A day built from hourly prices gets a real body and close to real extremes; an
    /// hour built from a single hourly price gets a body and no wick, because a wick is
    /// information the series doesn't carry. Volume isn't carried either — CoinGecko
    /// reports a rolling 24h figure, which is not this candle's.
    ///
    /// Expects ascending `time`.
    static func candles(from samples: [(time: Date, price: Double)],
                        interval: TimeInterval) -> [KlineData] {
        guard interval > 0, !samples.isEmpty else { return [] }

        var candles: [KlineData] = []
        var run: [Double] = []
        var currentBucket: Date?

        func flush() {
            guard let start = currentBucket, let close = run.last else { return }
            let open = candles.last?.closePrice ?? run[0]
            candles.append(
                KlineData(
                    openTime: start,
                    openPrice: open,
                    highPrice: max(run.max() ?? open, open),
                    lowPrice: min(run.min() ?? open, open),
                    closePrice: close,
                    volume: 0
                )
            )
        }

        for sample in samples {
            let start = bucketStart(of: sample.time, interval: interval)
            if start != currentBucket {
                flush()
                run.removeAll(keepingCapacity: true)
                currentBucket = start
            }
            run.append(sample.price)
        }
        flush()

        return candles
    }

    /// Where the candle of `interval` containing `date` opens.
    ///
    /// Epoch multiples for anything up to a day, which is how Binance aligns minute,
    /// hour and day klines. Weeks and months don't divide the epoch evenly, so those go
    /// through the calendar instead: Binance opens a weekly kline on Monday and a
    /// monthly one on the 1st, while epoch multiples would put every week on a Thursday
    /// and every "month" 30 days after the last one. Two charts side by side have to
    /// agree on which week a candle is.
    static func bucketStart(of date: Date, interval: TimeInterval) -> Date {
        switch interval {
        case ..<604_800:
            return Date(
                timeIntervalSince1970:
                    (date.timeIntervalSince1970 / interval).rounded(.down) * interval
            )
        case ..<2_419_200:  // a week, up to the shortest month
            return utcCalendar.dateInterval(of: .weekOfYear, for: date)?.start ?? date
        default:
            return utcCalendar.dateInterval(of: .month, for: date)?.start ?? date
        }
    }

    /// Weeks start on Monday, matching Binance's weekly klines rather than the
    /// locale's idea of a first day.
    private static let utcCalendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        calendar.firstWeekday = 2
        return calendar
    }()

    // MARK: - Type-flexible parsers

    /// Extract a timestamp in milliseconds from Int64, Double, or NSNumber.
    static func extractTimestamp(_ value: Any) -> Double? {
        if let v = value as? Int64 { return Double(v) }
        if let v = value as? Double { return v }
        if let v = value as? NSNumber { return v.doubleValue }
        return nil
    }

    /// Extract a Double from String, Double, or NSNumber.
    static func extractDouble(_ value: Any) -> Double? {
        if let v = value as? String { return Double(v) }
        if let v = value as? Double { return v }
        if let v = value as? NSNumber { return v.doubleValue }
        return nil
    }
}

extension Array where Element == KlineData {
    /// Percentage change from first to last close price. Nil if fewer than 2 points.
    var priceChangePercent: Double? {
        guard count >= 2, let first = first, let last = last, first.closePrice != 0 else { return nil }
        return ((last.closePrice - first.closePrice) / first.closePrice) * 100
    }

    /// Signed price movement from first to last close. Nil if fewer than 2 points.
    var priceChangeAmount: Double? {
        guard count >= 2, let first = first, let last = last else { return nil }
        return last.closePrice - first.closePrice
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
