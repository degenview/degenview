import Foundation

/// When a price still counts as current: how recently DegenView last got it from the provider, not when
/// the market last traded. A thin coin or a closed stock can go hours between trades and its last price is
/// still the current one; what matters is that we are not showing something we failed to refresh.
///
/// Each window is a few poll intervals (see `WatchlistQuoteCoordinator.Configuration`), so one missed round
/// does not dim a row but a real outage does.
enum WatchlistFreshness {
    /// Seconds between refreshes of a source while a sidebar is visible.
    static func refreshInterval(for source: DataSourceType) -> TimeInterval {
        switch source {
        case .binance, .coinbase, .alpaca: 5
        case .dexscreener: 10
        case .coingecko, .polymarket, .kalshi: 30
        case .coinMarketCap: 0
        }
    }

    /// How long after its last refresh a price is still shown as current.
    static func maximumAge(for source: DataSourceType) -> TimeInterval {
        switch source {
        case .binance, .alpaca: 45
        case .dexscreener: 60
        // Coinbase is refreshed from REST every 30 seconds when the socket has been quiet.
        case .coinbase: 90
        case .coingecko, .polymarket, .kalshi: 120
        case .coinMarketCap: 0
        }
    }

    /// `.stale` once the last refresh is older than the source's window; a stock outside US hours reads
    /// `.marketClosed` (its last price is the close, and still current); otherwise `.live`.
    static func evaluate(source: DataSourceType, receivedAt: Date, now: Date) -> WatchlistQuote.Freshness {
        guard now.timeIntervalSince(receivedAt) <= maximumAge(for: source) else { return .stale }
        if source == .alpaca, !isUSMarketOpen(at: now) { return .marketClosed }
        return .live
    }

    /// Regular US session, Monday to Friday 9:30 to 16:00 Eastern. Holidays are not modelled, so on one a
    /// stock reads "live" at its last close instead of "market closed".
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
