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
    /// Normal outer and per-card padding in the fixed-height grid.
    static let gridOuterPadding: CGFloat = 4
    static let gridCardPadding: CGFloat = 4

    /// Common plot height that lets all vertically stacked cards fit when possible.
    static func verticalPlotHeight(available: CGFloat, cardCount: Int) -> CGFloat {
        guard cardCount > 0 else { return available }
        let gaps = CGFloat(max(0, cardCount - 1)) * cardGap
        return (available - gaps) / CGFloat(cardCount) - cardChrome
    }

    /// Common plot height for a two-column grid.
    static func gridPlotHeight(available: CGFloat, cardCount: Int) -> CGFloat {
        max(0, gridCardHeight(available: available, cardCount: cardCount) - cardChrome)
    }

    /// Equal card height after accounting for all grid and item padding.
    static func gridCardHeight(available: CGFloat, cardCount: Int) -> CGFloat {
        guard cardCount > 0 else { return max(0, available) }
        let rowCount = (cardCount + gridColumns - 1) / gridColumns
        let padding = gridOuterInset(available: available, cardCount: cardCount) * 2
            + CGFloat(rowCount) * gridCardInset(available: available, cardCount: cardCount) * 2
        return max(0, (available - padding) / CGFloat(rowCount))
    }

    static func gridOuterInset(available: CGFloat, cardCount: Int) -> CGFloat {
        gridOuterPadding * gridPaddingScale(available: available, cardCount: cardCount)
    }

    static func gridCardInset(available: CGFloat, cardCount: Int) -> CGFloat {
        gridCardPadding * gridPaddingScale(available: available, cardCount: cardCount)
    }

    private static func gridPaddingScale(available: CGFloat, cardCount: Int) -> CGFloat {
        guard cardCount > 0 else { return 1 }
        let rowCount = (cardCount + gridColumns - 1) / gridColumns
        let nominal = gridOuterPadding * 2 + CGFloat(rowCount) * gridCardPadding * 2
        guard nominal > 0 else { return 1 }
        return min(1, max(0, available) / nominal)
    }

    /// Total height occupied by the rows and their padding.
    static func gridOccupancy(available: CGFloat, cardCount: Int) -> CGFloat {
        guard cardCount > 0 else { return 0 }
        let rowCount = (cardCount + gridColumns - 1) / gridColumns
        return gridOuterInset(available: available, cardCount: cardCount) * 2
            + CGFloat(rowCount) * (
                gridCardInset(available: available, cardCount: cardCount) * 2
                    + gridCardHeight(available: available, cardCount: cardCount)
            )
    }
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
    /// CoinGecko rate-limiter starting gap between calls (seconds).
    /// The public tier is throttled per rolling minute; the limiter widens this
    /// on every 429 and narrows it again while calls succeed.
    static let coingeckoRateLimitGap: TimeInterval = 4.0
    /// Widest the CoinGecko gap may grow after repeated 429s (seconds).
    static let coingeckoRateLimitMaxGap: TimeInterval = 20.0
    /// Gap multiplier applied on a 429.
    static let coingeckoRateLimitGrowth: Double = 1.6
    /// Gap multiplier applied after each successful call.
    static let coingeckoRateLimitDecay: Double = 0.92
    /// How long a sparkline prime stays usable for provisional candles (seconds).
    static let coingeckoProvisionalTTL: TimeInterval = 300
    /// CoinGecko coin-list cache staleness (seconds — 7 days).
    static let coinListStaleness: TimeInterval = 86400 * 7
    /// Auto-refresh interval for all charts (seconds).
    static let autoRefresh: TimeInterval = 5
    /// Debounce delay for ticker search (nanoseconds).
    static let searchDebounceNS: UInt64 = 300_000_000
    /// Stagger delay between individual ticker fetches at launch (nanoseconds).
    static let fetchStaggerNS: UInt64 = 100_000_000
    /// DEXScreener search result limit.
    static let dexSearchLimit = 20
    /// GeckoTerminal OHLC cache lifetime (seconds).
    static let geckoTerminalCacheTTL: TimeInterval = 120
    /// Minimum gap between GeckoTerminal calls — public tier allows ~30/min.
    static let geckoTerminalRateLimitGap: TimeInterval = 2.2
    /// Most candles GeckoTerminal returns from one OHLCV call.
    static let geckoTerminalMaxCandles = 1000
}

// MARK: - CoinGecko Constants

enum CoinGecko {
    /// Largest `per_page` the API accepts on `/coins/markets`.
    static let pageLimit = 250
}

// MARK: - Polymarket Constants

enum Polymarket {
    /// `limit_per_type` on `/public-search` — events returned, each expanding into
    /// its markets, so the flattened row count is a multiple of this.
    static let searchLimitPerType = 10
    /// Price-history cache TTL (seconds).
    static let cacheTTL: TimeInterval = 60
    /// Longest market title kept intact in a search row before truncation.
    static let maxTitleLength = 90
}

// MARK: - Icon Constants

enum Icon {
    /// Rendered icon edge length (points).
    static let size: CGFloat = 20
    /// How long a resolved icon URL stays usable (seconds — 7 days).
    static let cacheStaleness: TimeInterval = 7 * 86400
    /// How long an exhausted lookup is remembered before the chain runs again
    /// (seconds — 1 day). Without this, every miss re-walks all four sources on
    /// each card appearance.
    static let negativeTTL: TimeInterval = 86400
    /// Coins pulled into the market map that seeds symbol → icon lookups.
    static let maxCoins = 250
    /// Minimum spacing between market-snapshot attempts (seconds). Keeps a failed
    /// refresh from re-firing for every card that asks next.
    static let refreshRetryInterval: TimeInterval = 300
    /// How long a coin-id lookup waits for sibling cards to join its batch (nanoseconds).
    static let batchWindowNS: UInt64 = 150_000_000
    /// Longest string still plausibly a ticker — anything longer is a contract address.
    static let maxSymbolLength = 12
    /// Last-resort static icon set — no API, no rate limit, 404 on miss.
    /// Unmaintained since ~2021, so it only backstops long-established symbols.
    static let staticCDNBase = "https://raw.githubusercontent.com/spothq/cryptocurrency-icons/master/128/color"
    /// Community-maintained stock artwork, keyed by uppercase exchange ticker.
    /// Raw GitHub returns 404 when the repository has no matching company logo.
    static let stockCDNBase = "https://raw.githubusercontent.com/nvstly/icons/main/ticker_icons"
    /// Bump when stock resolution semantics change so cached crypto false-positives
    /// for Alpaca symbols are discarded once without flushing unrelated icons.
    static let stockResolverVersion = 2
}

// MARK: - Cache Constants

enum CacheLimit {
    /// Drop on-disk kline entries older than this on load (seconds — 3 days).
    static let diskStaleness: TimeInterval = 3 * 86400
    /// Max kline entries written to disk.
    static let maxDiskEntries = 120
    /// Debounce before flushing the kline cache to disk (nanoseconds).
    static let saveDebounceNS: UInt64 = 2_000_000_000
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

// MARK: - Indicator Constants

enum Indicator {
    /// Extra candles fetched beyond the visible window, so period-based indicators
    /// are already warmed up at the left edge instead of starting partway across.
    ///
    /// Sized to the longest offered EMA, and fetched whether or not any indicator is
    /// on: it costs one wider response rather than an extra request, and it doubles
    /// as slack that lets zooming redraw from data already in hand.
    static let warmupHeadroom = 200

    /// Periods offered for the EMA overlay.
    static let emaPeriods = [9, 20, 50, 100, 200]
    static let emaDefaultPeriod = 20

    /// Bollinger defaults — 20-period SMA, bands two standard deviations out.
    static let bollingerPeriod = 20
    static let bollingerMultiplier: Double = 2

    /// Supertrend defaults — Wilder ATR over 10 candles, offset by three ATRs.
    static let supertrendPeriod = 10
    static let supertrendMultiplier: Double = 3
}

// MARK: - RSI Constants

enum RSI {
    /// Lookback in candles. 14 is Wilder's original and what every chart defaults to.
    static let period = 14
    /// Guide levels drawn across the indicator strip.
    static let overbought: Double = 70
    static let oversold: Double = 30
}

// MARK: - Vertical Zoom Constants

/// Vertical price-scale zoom, applied on top of the auto-fitted price range.
enum PriceZoom {
    /// Widest price slice, relative to auto-fit.
    static let minFactor: Double = 0.25
    /// Narrowest price slice, relative to auto-fit.
    static let maxFactor: Double = 20
    /// Vertical drag distance in points that doubles or halves the zoom.
    /// Exponential, so the drag feels the same at 1x and at 10x.
    static let pointsPerDoubling: Double = 120
}

// MARK: - Drawing Tool Constants

/// Hand-drawn chart annotations. Sizes that only affect looks live on `ChartStyle`;
/// these are the interaction ones.
enum Drawing {
    /// How far a click may land from a handle or a line and still count as a hit.
    static let hitTolerance: CGFloat = 8
}

// MARK: - UI Constants

enum UI {
    /// Width of the vertical tool strip down the left edge of the window.
    static let toolSidebarWidth: CGFloat = 36
    /// Window minimum width. Includes the tool strip, so cards keep the width they
    /// had before it existed.
    static let windowMinWidth: CGFloat = 380 + toolSidebarWidth
    /// Window minimum height — toolbar plus one card at `chartMinHeight`.
    static let windowMinHeight: CGFloat = 420
    /// Window ideal size — landscape; charts read across time, not down it.
    static let windowIdealWidth: CGFloat = 1240
    static let windowIdealHeight: CGFloat = 800
    /// Fraction of the screen's visible frame a fresh window takes.
    static let windowDefaultWidthFraction: CGFloat = 0.78
    static let windowDefaultHeightFraction: CGFloat = 0.85
    /// Ceiling on the computed default, so a 5K display doesn't get a 2400 pt window.
    static let windowDefaultMaxWidth: CGFloat = 1440
    static let windowDefaultMaxHeight: CGFloat = 900
    /// Narrowest the default may be before height is trimmed to keep it landscape.
    static let windowMinAspect: CGFloat = 1.3
    /// UserDefaults key the main window's frame is remembered under.
    ///
    /// AppKit's own frame autosave is no use here: SwiftUI names the window after
    /// the scene's *load address*, so the key changes with every build and a
    /// remembered frame is never found again.
    static let windowFrameDefaultsKey = "charts.mainWindowFrame"
    /// How much of a remembered window must still land on an attached display for
    /// it to be reused — enough of the title bar to grab.
    static let windowRestoreMinVisibleWidth: CGFloat = 160
    static let windowRestoreMinVisibleHeight: CGFloat = 80
    /// Sheet frame width for Add Ticker.
    static let addTickerSheetWidth: CGFloat = 440
    /// Width of the optional favorites rail on the right.
    static let favoritesSidebarWidth: CGFloat = 260
    /// Sheet frame dimensions for Chart Settings.
    static let chartSettingsSheetWidth: CGFloat = 760
    static let chartSettingsSheetHeight: CGFloat = 680
    static let chartSettingsSheetMinWidth: CGFloat = 600
    static let chartSettingsSheetMinHeight: CGFloat = 520
    /// Search results max height in Add Ticker sheet.
    static let addTickerResultsMaxHeight: CGFloat = 300
    /// Search results min height in Add Ticker sheet.
    static let addTickerResultsMinHeight: CGFloat = 100
    /// Search results height range in Chart Settings sheet.
    static let chartSettingsResultsMinHeight: CGFloat = 80
    static let chartSettingsResultsMaxHeight: CGFloat = 180
    /// Suggestion grid columns.
    static let suggestionGridColumns = 5
    /// Market artwork edge length in a Polymarket search row.
    static let polymarketRowImageSize: CGFloat = 24
    /// Saved-view shortcut list on an empty tab.
    static let emptyStateViewListMaxHeight: CGFloat = 200
    static let emptyStateViewListWidth: CGFloat = 280
    /// Named view sentinel.
    static let unnamedView = "Unnamed"
}
