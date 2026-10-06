import SwiftUI

/// What the order will cost: value, margin, fee, the funds on hand, and how it changes the open
/// position. Turns red, with the shortfall, when the margin is more than the account has free.
struct PaperOrderSummaryCard: View {
    let draft: PaperOrderTicketDraft
    let currency: PaperCurrency

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            row("Order value", draft.notional.map(money) ?? "—")
            row("Required margin", draft.requiredMargin.map(money) ?? "—", color: draft.exceedsFunds ? .red : .primary)
            if let fee = draft.estimatedFee { row("Estimated fee", money(fee)) }
            row("Available funds", draft.availableFunds.map(money) ?? "—", color: draft.exceedsFunds ? .red : .primary)
            if let shortfall {
                Label(
                    "Short by \(money(shortfall)). Lower the quantity or raise leverage.",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .font(.caption)
                .foregroundStyle(.red)
                .fixedSize(horizontal: false, vertical: true)
            }
            if let note = positionNote {
                Divider()
                row("Position", note)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.4)))
    }

    private func row(_ title: String, _ value: String, color: Color = .primary) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title).font(.callout).foregroundStyle(.secondary)
            Spacer(minLength: 12)
            Text(value).font(.callout.weight(.medium).monospacedDigit()).foregroundStyle(color).lineLimit(1)
        }
        .accessibilityElement(children: .combine)
    }

    private var shortfall: Decimal? {
        guard draft.exceedsFunds, let required = draft.requiredMargin, let funds = draft.availableFunds else {
            return nil
        }
        return required - funds
    }

    /// How the order changes an open position in the same instrument; nil when there is none.
    private var positionNote: String? {
        let existing = draft.existingSignedQuantity
        guard existing != 0 else { return nil }
        let held = abs(existing)
        let direction = existing > 0 ? "long" : "short"
        let opposite = existing > 0 ? "short" : "long"
        guard let quantity = draft.quantity, quantity > 0 else { return "\(quantityText(held)) \(direction) open" }
        if draft.reducibleQuantity == 0 { return "Adds to \(direction) \(quantityText(held))" }
        if quantity == held { return "Closes \(direction) \(quantityText(held))" }
        if quantity < held { return "Reduces \(direction) by \(quantityText(quantity))" }
        return "Closes \(direction), opens \(opposite) \(quantityText(quantity - held))"
    }

    private func quantityText(_ value: Decimal) -> String {
        PaperTradingFormatter.quantity(value, instrument: draft.instrument)
    }

    private func money(_ value: Decimal) -> String {
        PaperTradingFormatter.money(value, currency: currency)
    }
}
