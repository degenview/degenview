import SwiftUI

/// "Add to Watchlist": every list with a check where the market already is, then New Watchlist….
/// Shared by the sidebar, chart cards and search results so membership reads the same everywhere.
struct WatchlistMembershipMenu: View {
    @ObservedObject var store: WatchlistStore
    let item: WatchlistInstrument
    let onNewWatchlist: (WatchlistInstrument) -> Void

    var body: some View {
        Menu("Add to Watchlist") {
            ForEach(store.lists) { list in
                Toggle(
                    list.name,
                    isOn: Binding(
                        get: { list.contains(item.instrument) },
                        set: { isMember in
                            if isMember {
                                try? store.addInstrument(item, to: list.id)
                            } else {
                                try? store.removeInstrument(item.instrument, from: list.id)
                            }
                        }))
            }
            Divider()
            Button("New Watchlist…") { onNewWatchlist(item) }
        }
        .disabled(store.loadFailed)
    }
}
