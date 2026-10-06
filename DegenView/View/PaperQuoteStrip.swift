import SwiftUI

/// The market's price across the top of the order ticket. Last is always shown; Bid, Ask and
/// Spread appear only for markets whose exchange stream supplies them, with the price the chosen
/// side would trade at (ask to buy, bid to sell) in that side's colour.
struct PaperQuoteStrip: View {
    let draft: PaperOrderTicketDraft

    private var hasBook: Bool { draft.bid != nil && draft.ask != nil }

    var body: some View {
        VStack(spacing: 6) {
            HStack(spacing: 0) {
                if hasBook {
                    cell("Bid", draft.bid, tint: draft.side == .sell ? draft.side.tint : nil)
                    divider
                }
                cell("Last", draft.last, tint: nil)
                if hasBook {
                    divider
                    cell("Ask", draft.ask, tint: draft.side == .buy ? draft.side.tint : nil)
                    if let spread = draft.spread {
                        divider
                        cell("Spread", spread, tint: nil)
                    }
                }
            }
            .padding(.vertical, 9)
            .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.4)))
            .accessibilityElement(children: .contain)
            if !hasBook {
                Text("Best bid and ask aren't available for this market, so orders fill at the last price.")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private var divider: some View {
        Divider().frame(height: 24)
    }

    private func cell(_ title: String, _ value: Decimal?, tint: Color?) -> some View {
        VStack(spacing: 2) {
            Text(title).font(.caption2).foregroundStyle(.secondary)
            Text(value.map { PaperTradingFormatter.price($0, instrument: draft.instrument) } ?? "—")
                .font(.callout.weight(.medium).monospacedDigit())
                .foregroundStyle(value == nil ? Color.secondary : (tint ?? .primary))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .combine)
    }
}
