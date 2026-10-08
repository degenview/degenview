import SwiftUI

/// Sizes the watchlist panel shares, so its parts line up by construction.
enum WatchlistMetrics {
    /// Where left-aligned content starts, measured from the panel's left edge: asset logos, section titles, the
    /// "Symbol" column title and the panel header all use this one value, so they share a left edge. It is narrower
    /// than the right edge so a flag's bookmark, which touches the border, sits behind the logo.
    static let leadingInset: CGFloat = 8

    /// Where right-aligned content ends, measured from the panel's right edge: column values, column titles,
    /// chevrons and the header buttons.
    static let trailingInset: CGFloat = 12

    /// Padding SwiftUI's `List` adds inside its table on each side of every row (measured by
    /// `WatchlistListMetricsTests`: half of the table's 17pt cell spacing, rounded either way). Rows ask the list
    /// for the difference, so their content lands on `leadingInset` / `trailingInset` instead of past them.
    static let listCellLeading: CGFloat = 8
    static let listCellTrailing: CGFloat = 9

    /// The inset to give a list row so its content starts at `leadingInset`: none, the list already pads it.
    static var rowLeadingInset: CGFloat { max(0, leadingInset - listCellLeading) }

    /// The inset to give a list row so its content ends at `trailingInset`.
    static var rowTrailingInset: CGFloat { max(0, trailingInset - listCellTrailing) }

    /// A flag's size. It is drawn from x = 0, touching the panel's left edge, and is narrower than `leadingInset`
    /// so it ends short of the asset logo with a small gap. It takes no layout space, so a flagged row's logo and
    /// text sit exactly where an unflagged row's do.
    static let flagMarkSize = CGSize(width: 6, height: 14)
}

extension View {
    /// The panel's standard left and right content insets.
    func watchlistHorizontalInsets() -> some View {
        padding(.leading, WatchlistMetrics.leadingInset).padding(.trailing, WatchlistMetrics.trailingInset)
    }
}
