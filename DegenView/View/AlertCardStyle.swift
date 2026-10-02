import SwiftUI

/// The card chrome shared by every alerts row: padding, a faint fill that lifts on hover, a hairline border.
struct AlertCardStyle: ViewModifier {
    @Binding var isHovered: Bool

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.primary.opacity(isHovered ? 0.07 : 0.035))
            )
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.5)))
            .contentShape(Rectangle())
            .onHover { hovering in withAnimation(.easeOut(duration: 0.12)) { isHovered = hovering } }
    }
}

extension View {
    func alertCard(isHovered: Binding<Bool>) -> some View {
        modifier(AlertCardStyle(isHovered: isHovered))
    }
}
