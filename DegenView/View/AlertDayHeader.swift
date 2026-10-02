import SwiftUI

/// A day's heading in a history list ("Today", "Yesterday", "Monday, 29 September") with how many fired.
struct AlertDayHeader: View {
    let title: String
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            Text(title).font(.subheadline.weight(.semibold))
            Text("\(count)")
                .font(.caption.weight(.semibold).monospacedDigit())
                .foregroundStyle(.secondary)
                .padding(.horizontal, 6)
                .padding(.vertical, 1)
                .background(.quaternary, in: Capsule())
            Spacer()
        }
        .foregroundStyle(.secondary)
        .padding(.top, 8)
        .padding(.bottom, 6)
        .background(Color(nsColor: .windowBackgroundColor).opacity(0.94))
        .accessibilityAddTraits(.isHeader)
    }
}
