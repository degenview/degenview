import SwiftUI

/// A transaction type as a small tinted pill: buys and sells read at a glance in a long table.
struct PortfolioTransactionTypeBadge: View {
    let type: PortfolioTransactionType

    var body: some View {
        Text(type.rawValue)
            .font(.caption.weight(.medium))
            .lineLimit(1)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .foregroundStyle(type.tint)
            .background(type.tint.opacity(0.14), in: Capsule())
    }
}

extension PortfolioTransactionType {
    var tint: Color {
        switch self {
        case .buy: .green
        case .sell: .red
        case .transferIn, .transferOut: .blue
        case .reward, .stakingReward, .airdrop, .mining, .interest: .purple
        case .fee: .orange
        case .adjustment: .gray
        }
    }
}
