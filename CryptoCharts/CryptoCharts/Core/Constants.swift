import Foundation

// MARK: - Layout Constants

enum ChartLayout {
    /// Gap between chart cards in vertical/grid layout.
    static let cardGap: CGFloat = 8
    /// Estimated non-chart chrome per card (header + padding + spacing).
    static let cardChrome: CGFloat = 55
    /// Grid layout column count.
    static let gridColumns = 2
    /// Grid layout column width fraction.
    static let gridColumnFraction: Double = 2.0
    /// Chart area minimum height.
    static let chartMinHeight: CGFloat = 50
}

// MARK: - Timeout Constants

enum Timeout {
    /// URLSession request timeout (seconds).
    static let request: Double = 10
    /// URLSession resource timeout (seconds).
    static let resource: Double = 30
    /// Binance kline cache TTL (seconds).
    static let binanceCacheTTL: TimeInterval = 15
    /// CoinGecko kline cache TTL (seconds — free tier, slow to update).
    static let coingeckoCacheTTL: TimeInterval = 120
    /// CoinGecko rate-limiter minimum gap between calls (seconds).
    static let coingeckoRateLimitGap: TimeInterval = 3.0
    /// CoinGecko coin-list cache staleness (seconds — 7 days).
    static let coinListStaleness: TimeInterval = 86400 * 7
    /// Icon cache staleness (seconds — 7 days).
    static let iconCacheStaleness: TimeInterval = 7 * 86400
    /// Auto-refresh interval for all charts (seconds).
    static let autoRefresh: TimeInterval = 5
    /// Debounce delay for ticker search (nanoseconds).
    static let searchDebounceNS: UInt64 = 300_000_000
    /// Stagger delay between individual ticker fetches at launch (nanoseconds).
    static let fetchStaggerNS: UInt64 = 100_000_000
    /// CoinGecko max coins fetched for icon cache.
    static let iconMaxCoins = 250
    /// DEXScreener search result limit.
    static let dexSearchLimit = 20
}

// MARK: - Format Constants

enum Format {
    /// Very small price threshold — use subscript zero-count notation below this.
    static let subscriptThreshold: Double = 0.001
    /// Scaling factor for subscript zero-count loop.
    static let subscriptScaleTarget: Double = 0.1
    /// Rounding overflow threshold for subscript notation.
    static let subscriptRoundingThreshold = 1_000
    /// Large number threshold — drop fraction digits beyond this.
    static let largeNumberThreshold: Double = 1_000_000
    /// Price digit thresholds for auto-precision.
    static let priceDigitThresholds: [(threshold: Double, digits: Int)] = [
        (1000, 0),
        (1, 2),
        (0.01, 4),
    ]
    /// Default digits when price is below all thresholds.
    static let defaultPriceDigits = 6
}

// MARK: - Candle Constants

enum Candle {
    /// Minimum candle count at max zoom-in.
    static let minCandles = 10
    /// Maximum candle count at max zoom-out.
    static let maxCandles = 500
    /// Zoom step fraction of current candle count.
    static let zoomStepFraction: Double = 0.1
}

// MARK: - UI Constants

enum UI {
    /// Window minimum width.
    static let windowMinWidth: CGFloat = 380
    /// Window ideal width.
    static let windowIdealWidth: CGFloat = 440
    /// Sheet frame width for Add Ticker.
    static let addTickerSheetWidth: CGFloat = 440
    /// Sheet frame dimensions for Chart Settings.
    static let chartSettingsSheetWidth: CGFloat = 420
    static let chartSettingsSheetHeight: CGFloat = 420
    /// Search results max height in Add Ticker sheet.
    static let addTickerResultsMaxHeight: CGFloat = 300
    /// Search results min height in Add Ticker sheet.
    static let addTickerResultsMinHeight: CGFloat = 100
    /// Search results max height in Chart Settings sheet.
    static let chartSettingsResultsMaxHeight: CGFloat = 180
    /// Suggestion grid columns.
    static let suggestionGridColumns = 5
    /// Named view sentinel.
    static let unnamedView = "Unnamed"
}
