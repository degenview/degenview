import SwiftUI

/// A rounded, inset-list card holding About rows separated by dividers.
struct AboutGroup<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(spacing: 0) { content }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(.separator.opacity(0.45), lineWidth: 1)
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

/// The hairline between two rows of an `AboutGroup`, inset to the row text.
struct AboutDivider: View {
    var body: some View {
        Divider().padding(.leading, AboutLayout.dividerInset)
    }
}
