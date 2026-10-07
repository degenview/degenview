import SwiftUI

/// A whole-row link in an `AboutGroup`: brand logo on its tile, title and hint, a hover highlight.
struct AboutLinkRow: View {
    let title: String
    let hint: String
    /// Asset-catalog name of the brand logo.
    let logo: String
    /// The tile the logo sits on, chosen so the logo reads in light and dark mode.
    let tile: Color
    let destination: URL
    @State private var hovering = false

    var body: some View {
        Link(destination: destination) {
            HStack(spacing: AboutLayout.badgeSpacing) {
                Image(logo)
                    .resizable()
                    .interpolation(.high)
                    .scaledToFit()
                    .padding(4)
                    .frame(width: AboutLayout.badgeSize, height: AboutLayout.badgeSize)
                    .background(tile, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 7, style: .continuous)
                            .stroke(.separator.opacity(0.6), lineWidth: 0.5)
                    }
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(.subheadline).foregroundStyle(.primary)
                    Text(hint).font(.caption).foregroundStyle(.secondary)
                }
                Spacer(minLength: 12)
                Image(systemName: "arrow.up.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(hovering ? .primary : .tertiary)
            }
            .padding(.horizontal, AboutLayout.rowPadding)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, minHeight: AboutLayout.rowHeight + 6, alignment: .leading)
            .background(hovering ? AnyShapeStyle(.quaternary.opacity(0.7)) : AnyShapeStyle(.clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(.easeOut(duration: 0.12), value: hovering)
        .help(destination.absoluteString)
        .accessibilityLabel("\(title), \(hint)")
    }
}
