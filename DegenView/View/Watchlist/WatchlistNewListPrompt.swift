import SwiftUI

/// Asks for a name, then creates a watchlist that already holds the market it was asked from.
private struct WatchlistNewListPrompt: ViewModifier {
    @Binding var instrument: WatchlistInstrument?
    @State private var name = ""
    @State private var failure: String?

    func body(content: Content) -> some View {
        content
            .environment(\.watchlistNewList) { item in
                name = ""
                instrument = item
            }
            .alert(
                "New Watchlist", isPresented: Binding(get: { instrument != nil }, set: { if !$0 { instrument = nil } }),
                presenting: instrument
            ) { item in
                TextField("Name", text: $name)
                Button("Create") { create(with: item) }
                Button("Cancel", role: .cancel) {}
            } message: { item in
                Text("Name the new watchlist. \(item.name) will be added to it.")
            }
            .alert("Watchlist", isPresented: Binding(get: { failure != nil }, set: { if !$0 { failure = nil } })) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(failure ?? "")
            }
    }

    private func create(with item: WatchlistInstrument) {
        let store = WatchlistStore.shared
        do {
            let created = try store.createWatchlist(name: name)
            try store.addInstrument(item, to: created.id)
        } catch {
            failure = error.localizedDescription
        }
    }
}

extension View {
    /// Makes "New Watchlist…" in the add-to-watchlist menus below this view ask for a name here.
    func watchlistNewListPrompt(for instrument: Binding<WatchlistInstrument?>) -> some View {
        modifier(WatchlistNewListPrompt(instrument: instrument))
    }
}
