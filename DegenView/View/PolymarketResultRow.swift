import SwiftUI

/// A single Polymarket market in the search list — one chartable bet.
///
/// Same selection contract as `SearchResultRow` (whole row is the hit target, accent
/// wash when picked), but leads with the market's own artwork and trails with the YES
/// probability instead of a USD price.
struct PolymarketResultRow: View {
    let result: TickerSearchResult
    var isSelected: Bool = false
    var onSelect: () -> Void = {}
    /// Non-nil enables checkbox mode — shows a toggle instead of row-selection highlight.
    var isChecked: Bool? = nil
    var onToggle: (() -> Void)? = nil
    var onCommit: (() -> Void)? = nil

    var body: some View {
        HStack(spacing: 8) {
            if let checked = isChecked {
                Image(systemName: checked ? "checkmark.square.fill" : "square")
                    .foregroundStyle(checked ? Color.accentColor : Color.secondary)
                    .font(.body)
            }

            TickerIconView(
                symbol: result.symbol,
                url: result.imageURL,
                size: UI.polymarketRowImageSize
            )

            Text(result.symbol)
                .font(.body.weight(.medium))
                .lineLimit(2)

            Spacer(minLength: 8)

            if let price = result.price {
                Text(PriceFormatter.format(price, scale: .probability))
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
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
        .modifier(PolymarketRowTapModifier(
            onSelect: { if isChecked != nil { onToggle?() } else { onSelect() } },
            onCommit: onCommit
        ))
        .background(
            isSelected
                ? Color.accentColor.opacity(0.15)
                : Color.clear
        )
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }
}

private struct PolymarketRowTapModifier: ViewModifier {
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
