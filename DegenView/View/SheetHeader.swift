import SwiftUI

/// The top of a sheet: a badge, a title and a one-line description. The badge is a tinted SF
/// Symbol by default, or any view (a chart's own market icon).
struct SheetHeader<Badge: View>: View {
    let title: String
    let subtitle: String
    @ViewBuilder let badge: Badge

    var body: some View {
        HStack(spacing: 14) {
            badge
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.title2.weight(.semibold)).lineLimit(1).truncationMode(.tail)
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
    }
}

extension SheetHeader where Badge == SheetHeaderSymbolBadge {
    init(systemImage: String, title: String, subtitle: String) {
        self.init(title: title, subtitle: subtitle) { SheetHeaderSymbolBadge(systemImage: systemImage) }
    }
}

/// The default `SheetHeader` badge: an accent-tinted SF Symbol in a rounded square.
struct SheetHeaderSymbolBadge: View {
    let systemImage: String

    var body: some View {
        Image(systemName: systemImage)
            .font(.system(size: 19, weight: .semibold))
            .foregroundStyle(Color.accentColor)
            .frame(width: 46, height: 46)
            .background(Color.accentColor.opacity(0.14), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}
