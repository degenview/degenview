import SwiftUI

private struct WatchlistNewListKey: EnvironmentKey {
    static let defaultValue: ((WatchlistInstrument) -> Void)? = nil
}

extension EnvironmentValues {
    /// Set by a screen that can ask for a new watchlist's name. Where it is nil, "Add to Watchlist"
    /// menus are not offered.
    var watchlistNewList: ((WatchlistInstrument) -> Void)? {
        get { self[WatchlistNewListKey.self] }
        set { self[WatchlistNewListKey.self] = newValue }
    }
}
