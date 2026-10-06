import SwiftUI

/// Closed round trips with gross, fees and net result.
struct PaperClosedTradesTable: View {
    @ObservedObject var store: PaperTradingStore

    var body: some View {
        if store.closedTrades.isEmpty {
            PaperEmptyState(
                systemImage: "checkmark.seal", title: "No closed trades",
                message: "A trade appears here once a position is closed.")
        } else {
            table
        }
    }

    private var table: some View {
        Table(store.closedTrades) {
            TableColumn("Symbol") { PaperSymbolCell(instrument: $0.instrument) }
                .width(min: 90, ideal: 120)
            TableColumn("Side") { PaperSideChip(side: $0.side) }
                .width(min: 56, ideal: 64)
            TableColumn("Size") { trade in
                PortfolioNumericCell(text: PaperTradingFormatter.quantity(trade.quantity, instrument: trade.instrument))
            }
            .width(min: 70, ideal: 90)
            TableColumn("Entry") { trade in
                PortfolioNumericCell(text: PaperTradingFormatter.price(trade.entryPrice, instrument: trade.instrument))
            }
            .width(min: 80, ideal: 100)
            TableColumn("Exit") { trade in
                PortfolioNumericCell(text: PaperTradingFormatter.price(trade.exitPrice, instrument: trade.instrument))
            }
            .width(min: 80, ideal: 100)
            TableColumn("Gross") { trade in
                PortfolioNumericCell(text: store.signedMoney(trade.grossPnL), color: PaperTradingStyle.pnl(trade.grossPnL))
            }
            .width(min: 85, ideal: 100)
            TableColumn("Fees") { trade in
                PortfolioNumericCell(text: trade.commission == 0 ? "—" : store.money(trade.commission), color: .secondary)
            }
            .width(min: 70, ideal: 80)
            TableColumn("Net") { trade in
                PortfolioNumericCell(text: store.signedMoney(trade.netPnL), color: PaperTradingStyle.pnl(trade.netPnL))
                    .fontWeight(.semibold)
            }
            .width(min: 90, ideal: 105)
            TableColumn("Closed") { trade in
                Text(PaperTimestamp.format(trade.exitTimestamp))
                    .monospacedDigit().foregroundStyle(.secondary).lineLimit(1)
            }
            .width(min: 120, ideal: 140)
        }
        .portfolioTableChrome(inset: 12)
        .padding(.vertical, 8)
    }
}
