import Foundation

/// What the sidebar can ask the tab it sits in to do with a market.
struct WatchlistInstrumentActions {
    var open: (WatchlistInstrument) -> Void
    var addChart: (WatchlistInstrument) -> Void
    var openInNewTab: (WatchlistInstrument) -> Void
}
