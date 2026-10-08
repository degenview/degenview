import SwiftUI

/// Sizes the watchlist panel shares, so its parts line up by construction.
enum WatchlistMetrics {
    /// Where left-aligned content starts: asset logos, section titles, the "Symbol" column title and the
    /// panel header all use this one value, so they share a left edge. It is narrower than the right edge so a
    /// flag's bookmark can sit behind the logo.
    static let leadingInset: CGFloat = 8

    /// Where right-aligned content ends: column values, chevrons and the header buttons.
    static let trailingInset: CGFloat = 12

    /// A flag's size. It is drawn in the leading gutter starting at x = 0, behind the logo, which begins at
    /// `leadingInset` and so covers the bookmark's flat end by a few points. It takes no layout space, so a
    /// flagged row's logo and text sit exactly where an unflagged row's do.
    static let flagMarkSize = CGSize(width: 11, height: 14)
}

extension View {
    /// The panel's standard left and right content insets.
    func watchlistHorizontalInsets() -> some View {
        padding(.leading, WatchlistMetrics.leadingInset).padding(.trailing, WatchlistMetrics.trailingInset)
    }
}
