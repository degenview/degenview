import SwiftUI

/// One labelled figure: a caption, the value, and an optional second line (a percentage, a date).
struct PortfolioStatCard: View {
    let title: String
    let value: String
    var tone: Color?
    var caption: String?
    var captionTone: Color?
    /// Value shown in a quieter style when there is nothing to report yet ("—", "Unavailable").
    var isPlaceholder = false
    /// Hidden by privacy mode: read out as "hidden" instead of the masked dots.
    var isMasked = false
    /// SF Symbol shown in a tinted badge beside the text. Nil leaves the card text-only.
    var systemImage: String?
    /// Badge color; defaults to the card's `tone`, else the accent color.
    var iconTint: Color?

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            if let systemImage {
                let tint = iconTint ?? tone ?? .accentColor
                Image(systemName: systemImage)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(tint)
                    .frame(width: 34, height: 34)
                    .background(tint.opacity(0.14), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                    .accessibilityHidden(true)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text(value)
                    .font(.title3.weight(.semibold))
                    .monospacedDigit()
                    .foregroundStyle(isPlaceholder ? Color.secondary : (tone ?? .primary))
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                // Always laid out, so cards with and without a caption share one height.
                Text(caption ?? " ")
                    .font(.caption)
                    .monospacedDigit()
                    .foregroundStyle(captionTone ?? tone ?? .secondary)
                    .lineLimit(1)
                    .opacity(caption == nil ? 0 : 1)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.4)))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue(isMasked ? "hidden" : [value, caption].compactMap { $0 }.joined(separator: ", "))
    }
}
