import SwiftUI

/// One line of an About group: a tinted symbol badge, a title, and trailing content.
struct AboutRow<Trailing: View>: View {
    let title: String
    let systemImage: String
    let tint: Color
    @ViewBuilder let trailing: Trailing

    var body: some View {
        HStack(spacing: AboutLayout.badgeSpacing) {
            Image(systemName: systemImage)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: AboutLayout.badgeSize, height: AboutLayout.badgeSize)
                .background(tint.gradient, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                .accessibilityHidden(true)
            Text(title)
            Spacer(minLength: 12)
            trailing
        }
        .font(.subheadline)
        .padding(.horizontal, AboutLayout.rowPadding)
        .frame(minHeight: AboutLayout.rowHeight)
    }
}
