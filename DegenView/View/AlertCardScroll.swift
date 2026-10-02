import SwiftUI

/// The scrolling stack every alerts tab lays its cards in: even gutters, spaced cards, day headers that pin.
struct AlertCardScroll<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 6, pinnedViews: [.sectionHeaders]) {
                content
            }
            .padding(.horizontal, 20)
            .padding(.top, 8)
            .padding(.bottom, 16)
        }
        .scrollBounceBehavior(.basedOnSize)
    }
}
