import SwiftUI

/// The account's numbers: equity and open P&L up front, the rest in a quieter row that scrolls
/// sideways instead of clipping when the window is narrow.
struct PaperMetricsStrip: View {
    let metrics: PaperAccountMetrics?
    let currency: PaperCurrency

    private struct Cell: Identifiable {
        let title: String
        let value: String
        var color: Color = .primary
        var help: String?
        var id: String { title }
    }

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 0) {
                hero
                ForEach(cells) { cell in
                    Divider().frame(height: 28).padding(.horizontal, 14)
                    cellView(cell)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    // MARK: Hero

    private var hero: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("Equity").font(.caption).foregroundStyle(.secondary)
            Text(metrics.map { money($0.equity) } ?? "—")
                .font(.title3.weight(.semibold).monospacedDigit())
                .foregroundStyle(metrics == nil ? Color.secondary : .primary)
                .lineLimit(1)
            unrealizedLine
        }
        .fixedSize()
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder private var unrealizedLine: some View {
        if let metrics {
            let pnl = metrics.unrealizedPnL
            let ratio = metrics.balance > 0 ? pnl / metrics.balance : nil
            HStack(spacing: 4) {
                Image(systemName: pnl > 0 ? "arrow.up.right" : (pnl < 0 ? "arrow.down.right" : "minus"))
                    .font(.system(size: 9, weight: .bold))
                Text(PaperTradingFormatter.signedMoney(pnl, currency: currency))
                if let ratio { Text("(\(PaperTradingFormatter.signedPercent(ratio)))") }
                Text("open").foregroundStyle(.secondary)
            }
            .font(.caption.monospacedDigit())
            .foregroundStyle(PaperTradingStyle.pnl(pnl))
        } else {
            Text(" ").font(.caption)
        }
    }

    // MARK: Cells

    private var cells: [Cell] {
        guard let metrics else {
            return ["Balance", "Realized P&L", "Available", "Margin used", "Margin buffer"].map {
                Cell(title: $0, value: "—", color: .secondary)
            }
        }
        return [
            Cell(title: "Balance", value: money(metrics.balance)),
            Cell(
                title: "Realized P&L", value: PaperTradingFormatter.signedMoney(metrics.realizedPnL, currency: currency),
                color: PaperTradingStyle.pnl(metrics.realizedPnL)),
            Cell(title: "Available", value: money(metrics.availableFunds)),
            Cell(
                title: "Margin used", value: money(metrics.positionMargin + metrics.ordersMargin),
                help: "Positions \(money(metrics.positionMargin)) · Orders \(money(metrics.ordersMargin))"),
            Cell(
                title: "Margin buffer", value: PaperTradingFormatter.percent(metrics.marginBuffer),
                color: PaperTradingStyle.marginBuffer(metrics.marginBuffer),
                help: "Available funds as a share of equity"),
        ]
    }

    private func cellView(_ cell: Cell) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(cell.title).font(.caption2).foregroundStyle(.secondary)
            Text(cell.value)
                .font(.callout.weight(.medium).monospacedDigit())
                .foregroundStyle(cell.color)
                .lineLimit(1)
        }
        .fixedSize()
        .help(cell.help ?? cell.title)
        .accessibilityElement(children: .combine)
    }

    private func money(_ value: Decimal) -> String {
        PaperTradingFormatter.money(value, currency: currency)
    }
}
