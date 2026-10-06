import SwiftUI

/// A replay-bar icon button. `label` is both tooltip and accessibility label; `shortcut` is shown
/// in the tooltip by the caller and bound here.
struct ReplayIconButton: View {
    let systemImage: String
    let label: String
    var shortcut: KeyboardShortcut?
    var tint: Color = .secondary
    let action: () -> Void
    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            PaperIconGlyph(systemImage: systemImage, tint: tint, isHovering: isHovering)
        }
        .buttonStyle(.plain)
        .modifier(OptionalShortcut(shortcut: shortcut))
        .onHover { isHovering = $0 }
        .help(label)
        .accessibilityLabel(label)
    }
}

private struct OptionalShortcut: ViewModifier {
    let shortcut: KeyboardShortcut?

    func body(content: Content) -> some View {
        if let shortcut {
            content.keyboardShortcut(shortcut)
        } else {
            content
        }
    }
}
