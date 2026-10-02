import Foundation

/// What one `request.security` call site remembers between chart bars. It lives in the runtime
/// state, so a realtime tick that is rolled back also rolls the site back.
struct PineSecuritySite {
    /// Open of the higher-timeframe bucket the chart bars are currently in.
    var bucket: Date?
    /// The confirmed chart bars of that bucket so far, folded into one candle.
    var accumulated: KlineData?
    /// Whether `accumulated` holds bars whose bucket has not been completed yet.
    var hasPendingBars = false
    /// Whether the history from before the chart's first bar has been fed in.
    var isWarmedUp = false
    /// Another symbol's series: how many of its candles have been completed and evaluated.
    var completedCandles = 0
    /// The expression's state as of the last completed higher-timeframe bar: histories, `var`s and the
    /// `ta.*` calls it made, none of which are shared with the chart script.
    var state = PineRuntimeState()
    /// The expression's value on the last completed higher-timeframe bar.
    var lastResult: PineRuntimeValue = .na
    /// The chart bar this site last answered, and what it answered, for a second call on that bar.
    var processedBar: Date?
    var barResult: PineRuntimeValue = .na
}
