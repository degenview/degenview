import SwiftUI

/// Open positions: size, entry, live mark, profit and loss, and Close / Reverse.
struct PaperPositionsTable: View {
    @ObservedObject var store: PaperTradingStore
    @State private var pendingReverse: PaperPosition?

    var body: some View {
        Group {
            if store.positions.isEmpty {
                PaperEmptyState(
                    systemImage: "chart.line.uptrend.xyaxis", title: "No open positions",
                    message: "Use BUY or SELL in a chart's header to open one.")
            } else {
                table
            }
        }
        .confirmationDialog(
            "Reverse \(pendingReverse?.instrument.symbol ?? "") position?",
            isPresented: Binding(get: { pendingReverse != nil }, set: { if !$0 { pendingReverse = nil } }),
            presenting: pendingReverse
        ) { position in
            Button("Reverse Position", role: .destructive) { Task { await store.reverse(position) } }
        } message: { position in
            Text(
                "Closes the \(position.side.label.lowercased()) and opens an equal "
                    + "\(position.side == .long ? "short" : "long") at market.")
        }
    }

    private var table: some View {
        Table(store.positions) {
            TableColumn("Symbol") { PaperSymbolCell(instrument: $0.instrument) }
                .width(min: 90, ideal: 120)
            TableColumn("Side") { PaperSideChip(side: $0.side) }
                .width(min: 56, ideal: 64)
            TableColumn("Size") { position in
                PortfolioNumericCell(text: PaperTradingFormatter.quantity(position.quantity, instrument: position.instrument))
            }
            .width(min: 70, ideal: 90)
            TableColumn("Entry") { position in
                PortfolioNumericCell(
                    text: PaperTradingFormatter.price(position.averageEntryPrice, instrument: position.instrument))
            }
            .width(min: 80, ideal: 100)
            TableColumn("Mark") { position in
                PortfolioNumericCell(
                    text: store.mark(for: position).map {
                        PaperTradingFormatter.price($0, instrument: position.instrument)
                    } ?? "—",
                    color: .secondary)
            }
            .width(min: 80, ideal: 100)
            TableColumn("Unrealized") { position in
                let pnl = store.unrealizedPnL(for: position)
                PortfolioNumericCell(text: store.signedMoney(pnl), color: PaperTradingStyle.pnl(pnl))
            }
            .width(min: 90, ideal: 105)
            TableColumn("Return") { position in
                if let ratio = store.returnRatio(for: position) {
                    PortfolioNumericCell(
                        text: PaperTradingFormatter.signedPercent(ratio), color: PaperTradingStyle.pnl(ratio))
                } else {
                    PortfolioNumericCell(text: "—", color: .secondary)
                }
            }
            .width(min: 70, ideal: 80)
            TableColumn("Realized") { position in
                let realized = position.realizedGrossPnL - position.commissions
                PortfolioNumericCell(text: store.signedMoney(realized), color: PaperTradingStyle.pnl(realized))
            }
            .width(min: 85, ideal: 100)
            TableColumn("") { position in
                HStack(spacing: 6) {
                    Spacer(minLength: 0)
                    Button("Close") { Task { await store.close(position) } }
                        .controlSize(.small)
                        .help("Close at market")
                    Menu {
                        Button("Reverse…") { pendingReverse = position }
                    } label: {
                        Image(systemName: "ellipsis")
                    }
                    .menuStyle(.borderlessButton)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .help("More actions")
                }
            }
            .width(min: 90, ideal: 96, max: 110)
        }
        .portfolioTableChrome(inset: 12)
        .padding(.vertical, 8)
    }
}
