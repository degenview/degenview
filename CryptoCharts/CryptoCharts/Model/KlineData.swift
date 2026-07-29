import Foundation

struct KlineData: Identifiable {
    let id = UUID()
    let openTime: Date
    let openPrice: Double
    var highPrice: Double
    var lowPrice: Double
    var closePrice: Double
    var volume: Double
}

extension KlineData {
    /// Parse from raw Binance kline array.
    /// Array layout: [openTime, open, high, low, close, volume, closeTime, quoteVolume, trades, takerBuyBase, takerBuyQuote, ignore]
    /// - openTime, closeTime: Int64 (ms since epoch)
    /// - prices (open, high, low, close): String
    /// - volumes (volume, quoteVolume, takerBuyBase, takerBuyQuote): String
    /// - trades: Int
    init?(raw: [Any]) {
        guard raw.count >= 6 else { return nil }

        // openTime — index 0, Int64 ms
        guard let ts = raw[0] as? Int64 else { return nil }
        self.openTime = Date(timeIntervalSince1970: Double(ts) / 1000.0)

        // Prices — indices 1-4, String → Double
        guard let openStr = raw[1] as? String, let open = Double(openStr),
              let highStr = raw[2] as? String, let high = Double(highStr),
              let lowStr  = raw[3] as? String, let low  = Double(lowStr),
              let closeStr = raw[4] as? String, let close = Double(closeStr)
        else { return nil }

        self.openPrice = open
        self.highPrice = high
        self.lowPrice = low
        self.closePrice = close

        // Volume — index 5, String → Double
        if let volStr = raw[5] as? String, let vol = Double(volStr) {
            self.volume = vol
        } else {
            self.volume = 0
        }
    }
}

extension Array where Element == KlineData {
    /// Percentage change from first to last close price. Nil if fewer than 2 points.
    var priceChangePercent: Double? {
        guard let first = first, let last = last, first.closePrice != 0 else { return nil }
        return ((last.closePrice - first.closePrice) / first.closePrice) * 100
    }
}
