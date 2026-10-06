import SwiftUI

/// A borderless icon button with a 28 pt hit target that lights up on hover. For dense rows where a
/// text button would crowd the layout; the tooltip doubles as the VoiceOver label.
struct SettingsIconButton: View {
    let systemImage: String
    let label: String
    var tint: Color?
    var isDestructive = false
    let action: () -> Void

    @Environment(\.isEnabled) private var isEnabled
    @State private var isHovering = false

    private var foreground: Color {
        if isHovering && isDestructive { return .red }
        return tint ?? (isHovering ? .primary : .secondary)
    }

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(foreground)
                .frame(width: 28, height: 28)
                .background(
                    (isDestructive ? Color.red : Color.primary).opacity(isHovering && isEnabled ? 0.08 : 0),
                    in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(isEnabled ? 1 : 0.4)
        .onHover { isHovering = $0 }
        .animation(.easeOut(duration: 0.12), value: isHovering)
        .help(label)
        .accessibilityLabel(label)
    }
}
