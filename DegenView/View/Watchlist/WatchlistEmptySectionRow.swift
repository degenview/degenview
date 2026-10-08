import SwiftUI

/// What an open section with no symbols shows: a dashed drop area on the same edges as every row.
struct WatchlistEmptySectionRow: View {
    var isDropTarget = false

    var body: some View {
        Text("Drag symbols here")
            .font(.caption)
            .foregroundStyle(isDropTarget ? Color.accentColor : Color.secondary.opacity(0.7))
            .frame(maxWidth: .infinity, minHeight: 28)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(isDropTarget ? Color.accentColor.opacity(0.12) : .clear)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .strokeBorder(
                        isDropTarget ? Color.accentColor : Color.secondary.opacity(0.3),
                        style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
            )
            .padding(.vertical, 2)
            .accessibilityHidden(true)
    }
}
