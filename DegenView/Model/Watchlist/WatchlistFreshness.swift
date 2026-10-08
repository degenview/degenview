import Foundation

/// When a quote still counts as current. The windows follow the alert engine's, loosened
/// where a quiet market legitimately goes minutes without a trade.
enum WatchlistFreshness {
    static func maximumAge(for source: DataSourceType) -> TimeInterval {
        switch source {
        case .binance: 180
        // The socket is the freshness signal; a thin product can go quiet for a while.
        case .coinbase: 3_600
        case .alpaca: 7_200
        case .coingecko, .dexscreener: 1_800
        // Polled every 30 seconds.
        case .polymarket, .kalshi: 180
        case .coinMarketCap: 0
        }
    }

    /// `.live` while within the source's window; afterwards `.marketClosed` for a stock outside
    /// US hours (an old last trade is expected then) and `.stale` for everything else.
    static func evaluate(source: DataSourceType, timestamp: Date, now: Date) -> WatchlistQuote.Freshness {
        guard now.timeIntervalSince(timestamp) > maximumAge(for: source) else { return .live }
        if source == .alpaca, !isUSMarketOpen(at: now) { return .marketClosed }
        return .stale
    }

    /// Regular US session, Monday to Friday 9:30 to 16:00 Eastern. Holidays are not modelled: on one,
    /// a stock reads "stale" instead of "market closed".
    static func isUSMarketOpen(at date: Date) -> Bool {
        var calendar = Calendar(identifier: .gregorian)
        guard let eastern = TimeZone(identifier: "America/New_York") else { return false }
        calendar.timeZone = eastern
        let parts = calendar.dateComponents([.weekday, .hour, .minute], from: date)
        guard let weekday = parts.weekday, (2...6).contains(weekday),
            let hour = parts.hour, let minute = parts.minute
        else { return false }
        let minutes = hour * 60 + minute
        return minutes >= 9 * 60 + 30 && minutes < 16 * 60
    }
}
