import SwiftUI

/// A section divider: click anywhere on it to fold or unfold.
struct WatchlistSectionRow: View {
    let section: WatchlistSection
    let count: Int
    let isDropTarget: Bool
    let onToggle: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "chevron.right")
                .font(.caption2.weight(.bold))
                .rotationEffect(.degrees(section.isCollapsed ? 0 : 90))
                .foregroundStyle(.secondary)
                .frame(width: 12)
            Text(section.title.uppercased())
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Text("\(count)")
                .font(.caption2)
                .foregroundStyle(.tertiary)
            Spacer()
        }
        .padding(.vertical, 3)
        .contentShape(Rectangle())
        .background(isDropTarget ? Color.accentColor.opacity(0.18) : .clear, in: RoundedRectangle(cornerRadius: 4))
        .onTapGesture(perform: onToggle)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(section.title), \(count) symbols, \(section.isCollapsed ? "collapsed" : "expanded")")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction(named: section.isCollapsed ? "Expand" : "Collapse", onToggle)
    }
}
