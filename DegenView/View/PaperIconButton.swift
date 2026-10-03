import SwiftUI

/// An icon-only button. `label` is both the tooltip and the accessibility label, so it is required.
struct PaperIconButton: View {
    let systemImage: String
    let label: String
    var tint: Color = .secondary
    var isActive = false
    let action: () -> Void
    @State private var isHovering = false

    var body: some View {
        Button(action: action) {
            PaperIconGlyph(systemImage: systemImage, tint: tint, isActive: isActive, isHovering: isHovering)
        }
        .buttonStyle(.plain)
        .onHover { isHovering = $0 }
        .help(label)
        .accessibilityLabel(label)
    }
}
