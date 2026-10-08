import SwiftUI

/// A flagged row's background: the flag's bookmark against the panel's left edge, the rest empty.
///
/// It is the row's *background*, which the list stretches across the whole row, edge to edge. The row's own content
/// starts inside the list's built-in side padding, so a mark drawn from there could never reach the border.
struct WatchlistFlagGutter: View {
    let flag: WatchlistFlag

    var body: some View {
        HStack(spacing: 0) {
            WatchlistFlagMark(flag: flag)
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .allowsHitTesting(false)
    }
}
