import SwiftUI

/// What an open section with no symbols shows: a quiet dashed area on the same edges as every row. Dropping a
/// symbol anywhere under the section's heading puts it in the section.
struct WatchlistEmptySectionRow: View {
    var body: some View {
        Text("Drag symbols here")
            .font(.caption)
            .foregroundStyle(Color.secondary.opacity(0.7))
            .frame(maxWidth: .infinity, minHeight: 28)
            .overlay(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .strokeBorder(Color.secondary.opacity(0.3), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
            )
            .padding(.vertical, 2)
            .accessibilityHidden(true)
    }
}
