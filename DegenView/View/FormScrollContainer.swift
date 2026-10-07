import SwiftUI

/// A sheet body that grows with its form up to `maxHeight`, then scrolls, with the action bar pinned
/// underneath. Sheets built from tall forms (an alert with a webhook message, a webhook with headers)
/// stay on screen, and Cancel / Create never scroll away.
struct FormScrollContainer<Content: View, Footer: View>: View {
    var maxHeight: CGFloat = 620
    @ViewBuilder let content: Content
    @ViewBuilder let footer: Footer

    @State private var contentHeight: CGFloat = 0

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                content
                    .padding(24)
                    .background(
                        GeometryReader { proxy in
                            Color.clear.preference(key: FormHeightKey.self, value: proxy.size.height)
                        })
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(height: contentHeight > 0 ? min(contentHeight, maxHeight) : nil)
            .onPreferenceChange(FormHeightKey.self) { contentHeight = $0 }

            Divider()
            footer
                .padding(.horizontal, 24)
                .padding(.vertical, 16)
        }
    }
}

private struct FormHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}
