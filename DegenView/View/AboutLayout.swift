import SwiftUI

/// Shared metrics for the About page's grouped rows, so the dividers line up with the text.
enum AboutLayout {
    static let rowHeight: CGFloat = 44
    static let rowPadding: CGFloat = 14
    static let badgeSize: CGFloat = 26
    static let badgeSpacing: CGFloat = 12
    /// Where a row's text begins: dividers start here, like an inset list.
    static let dividerInset = rowPadding + badgeSize + badgeSpacing
}
