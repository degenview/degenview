import SwiftUI

/// A colour flag, drawn as a small bookmark on its side. In a row it sits in the panel's left gutter, left of the
/// asset logo, and takes no layout space.
struct WatchlistFlagMark: View {
    let flag: WatchlistFlag
    var size = WatchlistMetrics.flagMarkSize

    var body: some View {
        WatchlistFlagShape()
            .fill(flag.color)
            // A hairline edge keeps yellow readable on a light background and the others crisp on dark.
            .overlay(WatchlistFlagShape().stroke(Color.black.opacity(0.15), lineWidth: 0.5))
            .frame(width: size.width, height: size.height)
            .help("\(flag.title) flag")
            .accessibilityLabel("\(flag.title) flag")
    }
}

#Preview("Flags") {
    VStack(alignment: .leading, spacing: 10) {
        ForEach(WatchlistFlag.allCases) { flag in
            HStack(spacing: 8) {
                WatchlistFlagMark(flag: flag)
                Text(flag.title)
            }
        }
    }
    .padding(20)
}
