import SwiftUI

/// A small tinted tag: BUY / SELL, LONG / SHORT, TP / SL.
struct PaperSideChip: View {
    let text: String
    let tint: Color

    var body: some View {
        Text(text)
            .font(.system(size: 10.5, weight: .bold))
            .tracking(0.3)
            .foregroundStyle(tint)
            .lineLimit(1)
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(tint.opacity(0.14), in: Capsule())
    }
}

extension PaperSideChip {
    init(side: PaperOrderSide) { self.init(text: side.badgeText, tint: side.tint) }
    init(side: PaperPositionSide) { self.init(text: side.badgeText, tint: side.tint) }

    /// TP in green, SL in red; entries have no role chip.
    init?(role: PaperOrderRole) {
        switch role {
        case .entry: return nil
        case .takeProfit: self.init(text: role.badgeText, tint: PaperTradingStyle.buy)
        case .stopLoss: self.init(text: role.badgeText, tint: PaperTradingStyle.sell)
        }
    }
}
