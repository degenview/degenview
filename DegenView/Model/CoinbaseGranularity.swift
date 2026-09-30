import Foundation

/// How a Binance-vocabulary interval token ("1h", "1w", …) is served from Coinbase.
///
/// Coinbase Exchange only produces 1m, 5m, 15m, 1h, 6h and 1d candles. Weekly, monthly,
/// quarterly and yearly charts are built by folding daily candles into calendar buckets,
/// the same trick `GeckoTerminalService` uses above its own daily ceiling.
struct CoinbaseGranularity: Equatable {
    /// Candle size requested from Coinbase, in seconds.
    let source: Int
    /// Candle size shown on the chart, in seconds.
    let target: TimeInterval
    /// Upper bound on source candles that make up one displayed candle.
    let sourceCandlesPerTarget: Int

    /// Granularities the `/candles` endpoint accepts — anything else is "Unsupported granularity".
    static let supported: [Int] = [60, 300, 900, 3_600, 21_600, 86_400]

    /// Buckets one `start`/`end` request asks for. Coinbase rejects a span of more than 300
    /// with a 400, and treats `end` as inclusive — the margin keeps the span clear of that edge.
    static let pageBuckets = 290

    var needsAggregation: Bool { TimeInterval(source) != target }

    init?(interval: String) {
        switch interval {
        case "1m": self.init(native: 60)
        case "5m": self.init(native: 300)
        case "15m": self.init(native: 900)
        case "1h": self.init(native: 3_600)
        case "6h": self.init(native: 21_600)
        case "1d": self.init(native: 86_400)
        case "1w": self.init(source: 86_400, target: 604_800, sourceCandlesPerTarget: 7)
        case "1M": self.init(source: 86_400, target: 2_592_000, sourceCandlesPerTarget: 31)
        case "3M": self.init(source: 86_400, target: 7_776_000, sourceCandlesPerTarget: 92)
        case "1Y": self.init(source: 86_400, target: 31_536_000, sourceCandlesPerTarget: 366)
        default: return nil
        }
    }

    private init(native seconds: Int) {
        self.init(source: seconds, target: TimeInterval(seconds), sourceCandlesPerTarget: 1)
    }

    private init(source: Int, target: TimeInterval, sourceCandlesPerTarget: Int) {
        self.source = source
        self.target = target
        self.sourceCandlesPerTarget = sourceCandlesPerTarget
    }

    /// Open of the displayed candle containing `date`. Shared by REST folding and live ticks,
    /// so a streamed candle lands in the same bucket the next REST refresh will put it in.
    func bucketStart(of date: Date) -> Date {
        KlineData.bucketStart(of: date, interval: target)
    }

    /// End of the displayed candle that opens at `start`.
    func bucketEnd(after start: Date) -> Date {
        // Months, quarters and years aren't a fixed length — find where the next one opens.
        if target >= 2_419_200 {
            return bucketStart(of: start.addingTimeInterval(target * 1.2))
        }
        return start.addingTimeInterval(target)
    }
}
