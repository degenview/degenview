import SwiftUI

/// The glyph of a panel icon button: a 26×26 target that lights up on hover or when `isActive`.
/// Shared by `PaperIconButton` and the panel's menus so they match.
struct PaperIconGlyph: View {
    let systemImage: String
    var tint: Color = .secondary
    var isActive = false
    var isHovering = false

    var body: some View {
        Image(systemName: systemImage)
            .font(.system(size: 12.5, weight: .medium))
            .foregroundStyle(isActive ? Color.accentColor : tint)
            .frame(width: 26, height: 26)
            .background(
                RoundedRectangle(cornerRadius: 7, style: .continuous)
                    .fill(isActive ? Color.accentColor.opacity(0.14) : Color.primary.opacity(isHovering ? 0.08 : 0))
            )
            .contentShape(Rectangle())
    }
}
