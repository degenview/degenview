import SwiftUI

/// "Add to Watchlist" on a search result, offered where a screen has said it can name a new list.
private struct WatchlistResultContextMenu: ViewModifier {
    let result: TickerSearchResult
    @Environment(\.watchlistNewList) private var newList

    func body(content: Content) -> some View {
        if let newList {
            content.contextMenu {
                WatchlistMembershipMenu(
                    store: .shared, item: WatchlistInstrument(searchResult: result), onNewWatchlist: newList)
            }
        } else {
            content
        }
    }
}

extension View {
    func watchlistContextMenu(for result: TickerSearchResult) -> some View {
        modifier(WatchlistResultContextMenu(result: result))
    }
}
