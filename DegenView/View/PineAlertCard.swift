import SwiftUI

/// A script-alert row in the same card chrome as the price-alert rows.
struct PineAlertCard<Content: View>: View {
    /// Rings the card, to say "this is the one you asked for".
    var isHighlighted = false
    @ViewBuilder let content: Content
    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 12) { content }
            .alertCard(isHovered: $isHovered)
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(Color.accentColor, lineWidth: 1.5)
                    .opacity(isHighlighted ? 1 : 0)
                    .allowsHitTesting(false)
            )
            .animation(.easeOut(duration: 0.25), value: isHighlighted)
    }
}
