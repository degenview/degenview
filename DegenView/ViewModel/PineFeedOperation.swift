import Foundation

/// One step for a chart's Pine execution pipeline, processed strictly in order.
enum PineFeedOperation: Sendable {
    /// Compile if needed, build a fresh host and recalculate over the bars captured with the request.
    case rebuild
    /// A live candle observation.
    case ingest(PineMarketUpdate)
    /// A REST refresh to reconcile with committed bars.
    case sync([KlineData])
}
