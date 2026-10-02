import SwiftUI

/// The best or worst asset by return: icon, name, return percentage and total P&L.
struct PortfolioPerformerCard: View {
    let title: String
    let result: PortfolioStatistics.AssetResult?
    let formatter: PortfolioValueFormatter
    @ObservedObject var info: PortfolioAssetInfoViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            if let result {
                HStack(spacing: 10) {
                    TickerIconView(symbol: result.asset.displayTicker, url: info.iconURL(for: result.asset), size: 30)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(result.asset.displayTicker).font(.headline)
                        Text(info.subtitle(for: result.asset))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 8)
                    VStack(alignment: .trailing, spacing: 1) {
                        Text(result.pnlPercent.map { formatter.signedPercent($0) } ?? "—")
                            .font(.title3.weight(.semibold))
                            .monospacedDigit()
                            .foregroundStyle(formatter.tone(result.pnlPercent) ?? .primary)
                        Text(formatter.signedMoney(result.totalPnL))
                            .font(.caption)
                            .monospacedDigit()
                            .foregroundStyle(formatter.tone(result.totalPnL) ?? .secondary)
                    }
                }
            } else {
                Text("—").font(.title3.weight(.semibold)).foregroundStyle(.secondary)
                Text(" ").font(.caption)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.4)))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue(
            result.map {
                "\($0.asset.displayTicker), \($0.pnlPercent.map { formatter.signedPercent($0) } ?? ""), "
                    + (formatter.privacy ? "amount hidden" : formatter.signedMoney($0.totalPnL))
            } ?? "none")
    }
}
