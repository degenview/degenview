import AppKit
import SwiftUI

/// The actions for one market, used by both its context menu and its hover button.
struct WatchlistInstrumentMenu: View {
    @ObservedObject var store: WatchlistStore
    @ObservedObject var viewModel: WatchlistSidebarViewModel
    let item: WatchlistInstrument
    let actions: WatchlistInstrumentActions
    let onNewWatchlist: (WatchlistInstrument) -> Void
    let onCreateAlert: (WatchlistInstrument) -> Void

    var body: some View {
        Button("Open in Focused Chart") { actions.open(item) }
        Button("Add as New Chart") { actions.addChart(item) }
        Button("Open in New Tab") { actions.openInNewTab(item) }

        Divider()

        WatchlistMembershipMenu(store: store, item: item, onNewWatchlist: onNewWatchlist)

        if let list = viewModel.list, !list.sections.isEmpty {
            Menu("Move to Section") {
                Button("No Section") { move(item, to: nil, in: list) }
                ForEach(list.sections) { section in
                    Button(section.title) { move(item, to: section.id, in: list) }
                }
            }
        }

        Menu("Set Flag") {
            ForEach(WatchlistFlag.allCases) { flag in
                Toggle(
                    flag.title,
                    isOn: Binding(
                        get: { store.flag(for: item.instrument) == flag },
                        set: { isOn in viewModel.perform { try store.setFlag(isOn ? flag : nil, for: item.instrument) } }
                    ))
            }
            Divider()
            Button("Clear Flag") { viewModel.perform { try store.setFlag(nil, for: item.instrument) } }
                .disabled(store.flag(for: item.instrument) == nil)
        }

        if item.supportsPriceAlert {
            Button("Create Price Alert…") { onCreateAlert(item) }
        }

        Divider()

        Button("Copy Symbol") { WatchlistFileIO.copy(item.instrument.symbol) }
        Button("Copy Qualified Symbol") { WatchlistFileIO.copy(item.instrument.qualifiedSymbol) }

        Divider()

        Button("Remove from Watchlist", role: .destructive) { viewModel.remove(item) }
    }

    private func move(_ item: WatchlistInstrument, to sectionID: UUID?, in list: Watchlist) {
        viewModel.perform { try store.moveInstrument(item.id, toSection: sectionID, in: list.id) }
    }
}
