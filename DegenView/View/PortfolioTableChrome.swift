import SwiftUI

/// The look every portfolio table shares: zebra rows, a hairline rounded border, and a margin
/// so the table sits inside its container instead of against the window edge.
struct PortfolioTableChrome: ViewModifier {
    var horizontalInset: CGFloat = 16

    func body(content: Content) -> some View {
        content
            .tableStyle(.inset(alternatesRowBackgrounds: true))
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(.separator))
            .padding(.horizontal, horizontalInset)
    }
}

extension View {
    /// Styles a `Table` like the other portfolio tables. `inset` is the side margin
    /// (0 where the container already pads).
    func portfolioTableChrome(inset: CGFloat = 16) -> some View {
        modifier(PortfolioTableChrome(horizontalInset: inset))
    }
}
