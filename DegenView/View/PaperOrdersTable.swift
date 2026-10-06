import SwiftUI

/// Working orders: what is resting, how much has filled, and a cancel button.
struct PaperOrdersTable: View {
    @ObservedObject var store: PaperTradingStore

    var body: some View {
        if store.workingOrders.isEmpty {
            PaperEmptyState(
                systemImage: "list.bullet.rectangle", title: "No working orders",
                message: "Limit, stop and protective orders rest here until they fill.")
        } else {
            table
        }
    }

    private var table: some View {
        Table(store.workingOrders) {
            TableColumn("Symbol") { PaperSymbolCell(instrument: $0.instrument) }
                .width(min: 90, ideal: 120)
            TableColumn("Side") { PaperSideChip(side: $0.side) }
                .width(min: 56, ideal: 64)
            TableColumn("Type") { order in
                HStack(spacing: 6) {
                    Text(order.type.label).lineLimit(1)
                    if let chip = PaperSideChip(role: order.role) { chip }
                }
            }
            .width(min: 90, ideal: 120)
            TableColumn("Filled / Qty") { order in
                PortfolioNumericCell(
                    text: "\(quantity(order.filledQuantity, order)) / \(quantity(order.originalQuantity, order))")
            }
            .width(min: 90, ideal: 110)
            TableColumn("Limit") { order in
                PortfolioNumericCell(
                    text: priceText(order.limitPrice, order), color: order.limitPrice == nil ? .secondary : .primary)
            }
            .width(min: 80, ideal: 100)
            TableColumn("Stop") { order in
                PortfolioNumericCell(
                    text: priceText(order.stopPrice, order), color: order.stopPrice == nil ? .secondary : .primary)
            }
            .width(min: 80, ideal: 100)
            TableColumn("Status") { order in
                SettingsStatusBadge(text: order.status.label, tone: order.status.tone)
            }
            .width(min: 90, ideal: 100)
            TableColumn("") { order in
                HStack {
                    Spacer(minLength: 0)
                    PaperIconButton(
                        systemImage: "xmark",
                        label: "Cancel \(order.side.label.lowercased()) \(order.type.label.lowercased()) order"
                    ) {
                        Task { await store.cancel(order.id) }
                    }
                }
            }
            .width(min: 34, ideal: 36, max: 40)
        }
        .portfolioTableChrome(inset: 12)
        .padding(.vertical, 8)
    }

    private func quantity(_ value: Decimal, _ order: PaperOrder) -> String {
        PaperTradingFormatter.quantity(value, instrument: order.instrument)
    }

    private func priceText(_ value: Decimal?, _ order: PaperOrder) -> String {
        value.map { PaperTradingFormatter.price($0, instrument: order.instrument) } ?? "—"
    }
}
