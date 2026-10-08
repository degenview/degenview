import Foundation

extension WatchlistQuote {
    /// A provider's answer as a row value. Change is the provider's own reference (rolling 24h, or
    /// the previous close for stocks); with no usable reference the change stays nil rather than
    /// being guessed from other data.
    init(source: DataSourceType, quote: SourceQuote, receivedAt: Date) {
        let timestamp = quote.timestamp ?? receivedAt
        var change: Double?
        var percent: Double?
        if let previous = quote.previousDayPrice, previous.isFinite, quote.price.isFinite {
            if source.isPredictionMarket {
                change = quote.price - previous
                // Percentage points of probability.
                percent = change.map { $0 * 100 }
            } else if previous > 0 {
                change = quote.price - previous
                percent = (quote.price / previous - 1) * 100
            }
        }
        self.init(
            last: quote.price, change: change, changePercent: percent, changeBasis: Self.basis(for: source),
            volume: quote.volume24h, volumeKind: quote.volumeKind.map(Self.kind), timestamp: timestamp,
            freshness: WatchlistFreshness.evaluate(source: source, timestamp: timestamp, now: receivedAt))
    }

    /// A trade from the Coinbase ticker channel, which carries the 24h open and volume.
    init(coinbaseTick tick: CoinbaseTick, receivedAt: Date) {
        var volume: Double?
        var kind: WatchlistQuote.VolumeKind?
        if let base = tick.volume24h {
            let converted = CoinbaseAPIService.volume(productID: tick.productID, baseVolume: base, price: tick.price)
            volume = converted.value
            kind = Self.kind(converted.kind)
        }
        var change: Double?
        var percent: Double?
        if let open = tick.open24h, open > 0 {
            change = tick.price - open
            percent = (tick.price / open - 1) * 100
        }
        self.init(
            last: tick.price, change: change, changePercent: percent, changeBasis: .rolling24h, volume: volume,
            volumeKind: kind, timestamp: tick.time,
            freshness: WatchlistFreshness.evaluate(source: .coinbase, timestamp: receivedAt, now: receivedAt))
    }

    static func basis(for source: DataSourceType) -> ChangeBasis {
        switch source {
        case .binance, .coinbase, .coingecko, .dexscreener: return .rolling24h
        case .alpaca: return .previousClose
        case .polymarket, .kalshi, .coinMarketCap: return .window
        }
    }

    private static func kind(_ kind: SourceVolumeKind) -> VolumeKind {
        switch kind {
        case .quoteCurrency: return .quoteCurrency
        case .shares: return .shares
        case .base: return .base
        }
    }
}
