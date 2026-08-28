import SwiftUI

/// Shared search result row used by AddTickerSheet and ChartSettingsSheet.
struct SearchResultRow: View {
    let result: TickerSearchResult
    let isSelected: Bool
    let onSelect: () -> Void
    var onCommit: (() -> Void)? = nil

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(result.symbol)
                    .font(.body.weight(.medium))

                if let chain = result.chain, let dex = result.dex {
                    Text("\(chain.capitalized) · \(dex.capitalized)")
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

            if isSelected {
                Image(systemName: "checkmark")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Color.accentColor)
                    .accessibilityLabel("Selected")
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .modifier(ResultRowTapModifier(onSelect: onSelect, onCommit: onCommit))
        .background(
            isSelected
                ? Color.accentColor.opacity(0.15)
                : Color.clear
        )
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }
}

/// Delays the single-click action just long enough to distinguish it from a double-click.
/// Without an exclusive gesture, a double-click can select and commit independently.
private struct ResultRowTapModifier: ViewModifier {
    let onSelect: () -> Void
    let onCommit: (() -> Void)?

    func body(content: Content) -> some View {
        if let onCommit {
            content.gesture(
                TapGesture(count: 2)
                    .exclusively(before: TapGesture(count: 1))
                    .onEnded { value in
                        switch value {
                        case .first:
                            onCommit()
                        case .second:
                            onSelect()
                        }
                    }
            )
        } else {
            content.onTapGesture(perform: onSelect)
        }
    }
}
