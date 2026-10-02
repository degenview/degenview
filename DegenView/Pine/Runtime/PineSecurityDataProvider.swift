import Foundation

/// A series `request.security` asks for: a symbol as the script wrote it, at a bar length in seconds.
struct PineSecurityKey: Hashable, Sendable {
    var symbol: String
    var interval: TimeInterval
}

/// Supplies the candles `request.security` reads from a series the chart does not carry: another symbol,
/// or the chart's own symbol on a longer timeframe from before the chart's first bar.
///
/// The engine never fetches anything; whoever runs it decides what exists. A provider answers from data it
/// already holds, so a run stays deterministic and synchronous.
protocol PineSecurityDataProvider: Sendable {
    /// Closed candles of `key`, oldest first and each opening on its bar boundary, or nil when the provider
    /// has nothing for it. The last candle may still be forming.
    func candles(for key: PineSecurityKey) -> [KlineData]?
}
