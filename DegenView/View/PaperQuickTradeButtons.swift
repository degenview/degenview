import SwiftUI

/// SELL and BUY in a chart card's header: two tinted pills on one line, each with the price an
/// order would be opened at. They open the order ticket; nothing is placed from here.
struct PaperQuickTradeButtons: View {
    let priceText: String
    let title: String
    let onSell: () -> Void
    let onBuy: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            TradeButton(
                side: .sell, priceText: priceText, label: "Sell \(title), paper order at last price \(priceText)",
                action: onSell)
            TradeButton(
                side: .buy, priceText: priceText, label: "Buy \(title), paper order at last price \(priceText)",
                action: onBuy)
        }
    }

    private struct TradeButton: View {
        let side: PaperOrderSide
        let priceText: String
        let label: String
        let action: () -> Void
        @State private var isHovering = false

        var body: some View {
            Button(action: action) {
                HStack(spacing: 5) {
                    Text(side.badgeText).font(.system(size: 10.5, weight: .bold)).tracking(0.3)
                    Text(priceText).font(.system(size: 12, weight: .semibold).monospacedDigit())
                }
                .foregroundStyle(side.tint)
                .lineLimit(1)
                .padding(.horizontal, 9)
                .frame(height: 24)
                .background(
                    side.tint.opacity(isHovering ? 0.24 : 0.14),
                    in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            }
            .buttonStyle(.plain)
            .onHover { isHovering = $0 }
            .accessibilityLabel(label)
            .help("\(side.label) \(priceText) — opens a paper order ticket")
        }
    }
}
