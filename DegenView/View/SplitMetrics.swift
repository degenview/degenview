import CoreGraphics

/// The sizing rules of a two-pane split, kept free of SwiftUI so they can be tested.
///
/// One pane is the *secondary*: it has an explicit length (a sidebar, a chart, a drawer) that the
/// user drags. The *primary* pane takes whatever is left, and never shrinks below its minimum.
struct SplitMetrics: Equatable {
    /// Length of the whole container along the split axis.
    var total: CGFloat
    var minPrimary: CGFloat
    var minSecondary: CGFloat
    var maxSecondary: CGFloat?
    /// Share of `total` the secondary pane gets when the user hasn't resized it.
    var defaultFraction: CGFloat
    /// A fixed length for that case instead, for panes with an ideal size (a sidebar).
    var defaultLength: CGFloat?

    /// The secondary pane's length for a stored preference. Zero or negative means "never
    /// resized": use the default.
    func secondaryLength(for stored: CGFloat) -> CGFloat {
        clamped(stored > 0 ? stored : defaultLength ?? total * defaultFraction)
    }

    /// Keeps the secondary pane between its own limits and leaves the primary its minimum. When
    /// the container is too small for both minimums, the primary's minimum yields first.
    func clamped(_ length: CGFloat) -> CGFloat {
        let upper = max(0, min(maxSecondary ?? .infinity, total - minPrimary))
        let lower = min(minSecondary, upper)
        return min(max(length, lower), upper)
    }

    /// The stored length after the divider moves by `delta` points, where `delta` is positive
    /// when it moves in the direction that grows the secondary pane.
    func resized(from stored: CGFloat, by delta: CGFloat) -> CGFloat {
        clamped(secondaryLength(for: stored) + delta)
    }
}
