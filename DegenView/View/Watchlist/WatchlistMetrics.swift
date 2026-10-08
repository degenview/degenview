import CoreGraphics

/// Sizes the watchlist panel shares, so its parts line up by construction.
enum WatchlistMetrics {
    /// Space between the panel's edge and its content. The header, filter field, column titles, every
    /// row and every section title use this one value, so symbols, section titles and column heads share
    /// a left edge and values and chevrons share a right edge.
    static let edgeInset: CGFloat = 12

    /// A flag's size. It is drawn in the leading gutter (x = 0), narrower than `edgeInset`, so a flagged row's logo
    /// and text sit exactly where an unflagged row's do.
    static let flagMarkSize = CGSize(width: 10, height: 8)
}
