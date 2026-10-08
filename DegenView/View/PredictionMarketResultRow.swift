import SwiftUI

/// A single prediction market in the search list — one chartable bet.
///
/// Same selection contract as `SearchResultRow` (whole row is the hit target, accent
/// wash when picked), but leads with the market's own artwork and trails with the YES
/// probability instead of a USD price.
struct PredictionMarketResultRow: View {
    let result: TickerSearchResult
    var isSelected: Bool = false
    var onSelect: () -> Void = {}
    /// Non-nil enables checkbox mode — shows a toggle instead of row-selection highlight.
    var isChecked: Bool? = nil
    var onToggle: (() -> Void)? = nil
    var onCommit: (() -> Void)? = nil

    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 10) {
            if let checked = isChecked {
                Image(systemName: checked ? "checkmark.square.fill" : "square")
                    .foregroundStyle(checked ? Color.accentColor : Color.secondary)
                    .font(.body)
            }

            TickerIconView(
                symbol: result.symbol,
                url: result.imageURL,
                size: UI.predictionMarketRowImageSize
            )

            Text(result.symbol)
                .font(.body.weight(.medium))
                .lineLimit(2)

            Spacer(minLength: 8)

            if let price = result.price {
                Text(PriceFormatter.format(price, scale: .probability))
                    .font(.callout.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }

            if isChecked == nil {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(Color.accentColor)
                    .opacity(isSelected ? 1 : 0)
                    .accessibilityLabel("Selected")
                    .accessibilityHidden(!isSelected)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .modifier(PredictionMarketRowTapModifier(
            onSelect: { if isChecked != nil { onToggle?() } else { onSelect() } },
            onCommit: onCommit
        ))
        .background(rowFill, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(isSelected ? Color.accentColor.opacity(0.5) : .clear)
        )
        .onHover { isHovered = $0 }
        .watchlistContextMenu(for: result)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var rowFill: Color {
        if isSelected { return Color.accentColor.opacity(0.12) }
        return isHovered ? Color.primary.opacity(0.06) : .clear
    }
}

private struct PredictionMarketRowTapModifier: ViewModifier {
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
