import Foundation

/// One observation of a candle from a market-data source, before the aggregator has judged it.
struct PineMarketUpdate: Sendable {
    enum Origin: Sendable {
        /// A live push (WebSocket, live bar): the freshest data there is.
        case stream
        /// A REST refresh: may lag a stream, and carries no close flag.
        case poll
    }

    var bar: KlineData
    var origin: Origin
}
