import SwiftUI

/// The bar under the panel's table: a one-line summary of the section and its bulk action
/// (Close All, Cancel All, Export). It sits outside the table, so it stays in the empty states.
struct PaperPanelFooter: View {
    @ObservedObject var store: PaperTradingStore
    let tab: PaperManagerTab
    let onExport: () -> Void
    @State private var confirmCloseAll = false
    @State private var confirmCancelAll = false

    var body: some View {
        AlertFooterBar(systemImage: tab.systemImage, note: note) { action }
            .confirmationDialog("Close all positions?", isPresented: $confirmCloseAll) {
                Button("Close \(count(store.positions.count, "Position"))", role: .destructive) {
                    let positions = store.positions
                    Task { for position in positions { await store.close(position) } }
                }
            } message: {
                Text("Each position is closed at market.")
            }
            .confirmationDialog("Cancel all working orders?", isPresented: $confirmCancelAll) {
                Button("Cancel \(count(store.workingOrders.count, "Order"))", role: .destructive) {
                    let orders = store.workingOrders
                    Task { for order in orders { await store.cancel(order.id) } }
                }
            } message: {
                Text("Protective take-profit and stop-loss orders are canceled too.")
            }
    }

    // MARK: Note

    private var note: String {
        switch tab {
        case .positions:
            let positions = store.positions
            guard !positions.isEmpty else { return "No open positions" }
            let pnl = positions.reduce(Decimal.zero) { $0 + store.unrealizedPnL(for: $1) }
            return "\(positions.count) open · Unrealized \(store.signedMoney(pnl))"
        case .orders:
            let orders = store.workingOrders.count
            return orders == 0 ? "No working orders" : "\(orders) working"
        case .history:
            let events = store.orderHistory.count
            return events == 0 ? "No order events" : count(events, "event")
        case .accountHistory:
            let trades = store.closedTrades
            guard let winRate = trades.winRate else { return "No closed trades" }
            let percent = Int((winRate * 100).doubleValue.rounded())
            return "\(count(trades.count, "trade")) · Win rate \(percent)% · Net \(store.signedMoney(trades.netPnL))"
        case .journal:
            let entries = store.journal.count
            return entries == 0 ? "No journal entries" : count(entries, "entry", plural: "entries")
        }
    }

    private func count(_ value: Int, _ noun: String, plural: String? = nil) -> String {
        value == 1 ? "1 \(noun.lowercased())" : "\(value) \((plural ?? noun + "s").lowercased())"
    }

    // MARK: Action

    @ViewBuilder private var action: some View {
        switch tab {
        case .positions:
            Button("Close All") { confirmCloseAll = true }
                .controlSize(.small)
                .disabled(store.positions.isEmpty)
        case .orders:
            Button("Cancel All") { confirmCancelAll = true }
                .controlSize(.small)
                .disabled(store.workingOrders.isEmpty)
        case .history, .accountHistory, .journal:
            Button(action: onExport) {
                Label("Export…", systemImage: "square.and.arrow.up")
            }
            .controlSize(.small)
        }
    }
}
