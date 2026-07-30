import SwiftUI

/// Shared search result row used by AddTickerSheet and ChartSettingsSheet.
struct SearchResultRow: View {
    let result: TickerSearchResult
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(result.symbol)
                    .font(.body.weight(.medium))

                if let chain = result.chain, let dex = result.dex {
                    Text("\(chain.capitalized) · \(dex.capitalized)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else if result.source == .coingecko {
                    Text("via CoinGecko")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                } else if result.source == .binance {
                    Text("via Binance")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            Spacer()

            if let price = result.price {
                Text(price, format: .currency(code: "USD").precision(.fractionLength(2...6)))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .onTapGesture { onSelect() }
        .background(isSelected
            ? Color.accentColor.opacity(0.15)
            : Color.clear
        )
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }
}
