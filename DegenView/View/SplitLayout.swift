import SwiftUI

/// Places two subviews — primary first, secondary second — side by side or stacked.
///
/// A real `Layout` rather than an `HStack`/`VStack` switch: the subviews keep their identity
/// when the axis or order changes, so an `NSTextView` keeps its cursor, scroll offset and undo
/// stack as the chart moves from the left of the editor to below it.
struct SplitLayout: Layout {
    var axis: Axis
    /// Whether the secondary pane sits before (left of, above) the primary.
    var secondaryFirst: Bool
    var storedLength: CGFloat
    var metrics: SplitMetrics
    /// 1 when the secondary pane is shown, 0 when collapsed. Animatable, so it slides.
    var reveal: CGFloat

    var animatableData: CGFloat {
        get { reveal }
        set { reveal = newValue }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        CGSize(width: proposal.width ?? 400, height: proposal.height ?? 300)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 2 else { return }
        let horizontal = axis == .horizontal
        var metrics = metrics
        metrics.total = horizontal ? bounds.width : bounds.height

        let full = metrics.secondaryLength(for: storedLength)
        let shown = full * reveal
        let primaryLength = metrics.total - shown
        let cross = horizontal ? bounds.height : bounds.width

        // A collapsed secondary pane keeps its full size and slides out under the container's clip,
        // so it isn't squeezed to nothing mid-animation.
        let primaryOffset = secondaryFirst ? shown : 0
        let secondaryOffset = secondaryFirst ? shown - full : metrics.total - shown

        place(subviews[0], offset: primaryOffset, length: primaryLength, cross: cross, in: bounds)
        place(subviews[1], offset: secondaryOffset, length: full, cross: cross, in: bounds)
    }

    private func place(
        _ subview: LayoutSubview, offset: CGFloat, length: CGFloat, cross: CGFloat, in bounds: CGRect
    ) {
        let horizontal = axis == .horizontal
        subview.place(
            at: CGPoint(
                x: bounds.minX + (horizontal ? offset : 0),
                y: bounds.minY + (horizontal ? 0 : offset)),
            anchor: .topLeading,
            proposal: ProposedViewSize(
                width: horizontal ? max(length, 0) : cross,
                height: horizontal ? cross : max(length, 0)))
    }
}
