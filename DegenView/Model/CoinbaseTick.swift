import Foundation

/// One trade from Coinbase's `ticker` channel.
///
/// Coinbase has no candle stream, so the chart builds its live candle from these.
struct CoinbaseTick: Equatable {
    let productID: String
    let price: Double
    /// Base-asset size of the trade.
    let size: Double
    let time: Date
    let tradeID: Int64
    /// The best bid and ask the frame reports alongside the trade; nil when it carries none.
    let bestBid: Double?
    let bestAsk: Double?

    /// Decode a `ticker` frame. Returns nil for every other message type —
    /// `subscriptions`, `heartbeat` and `error` frames share the socket.
    init?(json: [String: Any]) {
        guard json["type"] as? String == "ticker",
            let productID = json["product_id"] as? String,
            let price = (json["price"] as? String).flatMap(Double.init),
            let tradeID = (json["trade_id"] as? NSNumber)?.int64Value,
            let time = (json["time"] as? String).flatMap(Self.parseTime)
        else { return nil }

        self.productID = productID
        self.price = price
        self.size = (json["last_size"] as? String).flatMap(Double.init) ?? 0
        self.time = time
        self.tradeID = tradeID
        self.bestBid = (json["best_bid"] as? String).flatMap(Double.init)
        self.bestAsk = (json["best_ask"] as? String).flatMap(Double.init)
    }

    init(
        productID: String, price: Double, size: Double, time: Date, tradeID: Int64,
        bestBid: Double? = nil, bestAsk: Double? = nil
    ) {
        self.productID = productID
        self.price = price
        self.size = size
        self.time = time
        self.tradeID = tradeID
        self.bestBid = bestBid
        self.bestAsk = bestAsk
    }

    /// Coinbase stamps microseconds: `2026-09-30T17:28:25.629380Z`.
    private static func parseTime(_ text: String) -> Date? {
        formatter.date(from: text)
    }

    private static let formatter: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()
}
