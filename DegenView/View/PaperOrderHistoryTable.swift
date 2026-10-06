import SwiftUI

/// Every order event — placed, filled, modified, canceled — newest first.
struct PaperOrderHistoryTable: View {
    @ObservedObject var store: PaperTradingStore

    var body: some View {
        if store.orderHistory.isEmpty {
            PaperEmptyState(
                systemImage: "clock", title: "No order history",
                message: "Placed, filled and canceled orders are logged here.")
        } else {
            Table(store.orderHistory) {
                TableColumn("Time") { event in
                    Text(PaperTimestamp.format(event.timestamp))
                        .monospacedDigit().foregroundStyle(.secondary).lineLimit(1)
                }
                .width(min: 120, ideal: 140)
                TableColumn("Symbol") { PaperSymbolCell(instrument: $0.order.instrument) }
                    .width(min: 90, ideal: 120)
                TableColumn("Event") { event in
                    SettingsStatusBadge(text: event.kind.label, tone: event.kind.tone)
                }
                .width(min: 90, ideal: 110)
                TableColumn("Details") { event in
                    Text(event.message)
                        .lineLimit(1)
                        .help("\(event.message)\nOrder \(event.orderID.uuidString)")
                }
                .width(min: 200, ideal: 360)
            }
            .portfolioTableChrome(inset: 12)
            .padding(.vertical, 8)
        }
    }
}
