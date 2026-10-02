import SwiftUI

/// A selectable option as a card: icon badge, title, description and a radio mark.
///
/// Used where a sheet offers a handful of mutually exclusive choices — chart kinds,
/// CoinMarketCap indices — instead of a menu or radio group.
struct ChoiceCard: View {
    let title: String
    var subtitle: String?
    let systemImage: String
    let isSelected: Bool
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(isSelected ? Color.accentColor : .secondary)
                    .frame(width: 36, height: 36)
                    .background(
                        (isSelected ? Color.accentColor : Color.secondary).opacity(0.14),
                        in: RoundedRectangle(cornerRadius: 9, style: .continuous)
                    )
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.subheadline.weight(.semibold)).multilineTextAlignment(.leading)
                    if let subtitle {
                        Text(subtitle).font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.leading)
                    }
                }
                Spacer(minLength: 8)
                Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(isSelected ? Color.accentColor : .secondary.opacity(0.6))
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(fill, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(
                        isSelected ? Color.accentColor : Color.primary.opacity(0.12), lineWidth: isSelected ? 1.5 : 1)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .accessibilityLabel(title)
        .accessibilityHint(subtitle ?? "")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var fill: Color {
        if isSelected { return Color.accentColor.opacity(0.1) }
        return isHovered ? Color.primary.opacity(0.07) : Color.primary.opacity(0.03)
    }
}
