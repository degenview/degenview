import SwiftUI

/// A script-alert row in the same card chrome as the price-alert rows.
struct PineAlertCard<Content: View>: View {
    @ViewBuilder let content: Content
    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 12) { content }
            .alertCard(isHovered: $isHovered)
    }
}
