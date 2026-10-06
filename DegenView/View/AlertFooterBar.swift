import SwiftUI

/// The bar under an alerts list: a one-line note on the left, the tab's action on the right.
///
/// It sits outside the scrolling list, so it stays put in the empty and no-results states and the
/// cards never run beneath it.
struct AlertFooterBar<Action: View>: View {
    let systemImage: String
    let note: String
    @ViewBuilder let action: Action

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 12) {
                Label {
                    Text(note)
                } icon: {
                    Image(systemName: systemImage)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                Spacer(minLength: 12)
                action
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 10)
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }
}
