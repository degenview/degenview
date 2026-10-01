import AppKit
import SwiftUI

/// Two panes with a draggable divider. The secondary pane has a persisted length and can be
/// collapsed; the primary pane takes the rest.
///
/// The axis and the side the secondary pane sits on may change while the panes are on screen —
/// neither pane is rebuilt (see `SplitLayout`).
struct SplitContainer<Primary: View, Secondary: View>: View {
    var axis: Axis
    /// Whether the secondary pane sits before (left of, above) the primary.
    var secondaryFirst = false
    /// Points; zero until the user first drags the divider.
    @Binding var secondaryLength: Double
    var isSecondaryVisible = true
    var minPrimary: CGFloat
    var minSecondary: CGFloat
    var maxSecondary: CGFloat?
    var defaultFraction: CGFloat = 0.4
    var defaultLength: CGFloat?
    @ViewBuilder var primary: () -> Primary
    @ViewBuilder var secondary: () -> Secondary

    @State private var total: CGFloat = 0
    @State private var dragStart: CGFloat?

    private var metrics: SplitMetrics {
        SplitMetrics(
            total: total, minPrimary: minPrimary, minSecondary: minSecondary, maxSecondary: maxSecondary,
            defaultFraction: defaultFraction, defaultLength: defaultLength)
    }

    var body: some View {
        SplitLayout(
            axis: axis, secondaryFirst: secondaryFirst, storedLength: CGFloat(secondaryLength),
            metrics: metrics, reveal: isSecondaryVisible ? 1 : 0
        ) {
            primary()
            secondary()
                .overlay(alignment: handleAlignment) {
                    SplitHandle(
                        axis: axis, onDrag: drag, onEnd: { dragStart = nil }, onReset: { secondaryLength = 0 })
                        .offset(handleOffset)
                        .allowsHitTesting(isSecondaryVisible)
                        .opacity(isSecondaryVisible ? 1 : 0)
                }
                .allowsHitTesting(isSecondaryVisible)
                .accessibilityHidden(!isSecondaryVisible)
        }
        .clipped()
        .onGeometryChange(for: CGFloat.self) { proxy in
            axis == .horizontal ? proxy.size.width : proxy.size.height
        } action: {
            total = $0
        }
    }

    /// The secondary pane's edge that faces the primary pane.
    private var handleAlignment: Alignment {
        switch (axis, secondaryFirst) {
        case (.horizontal, true): .trailing
        case (.horizontal, false): .leading
        case (.vertical, true): .bottom
        case (.vertical, false): .top
        }
    }

    /// Centres the hit area on the boundary rather than inside the secondary pane.
    private var handleOffset: CGSize {
        let half = SplitHandle.hitThickness / 2
        switch (axis, secondaryFirst) {
        case (.horizontal, true): return CGSize(width: half, height: 0)
        case (.horizontal, false): return CGSize(width: -half, height: 0)
        case (.vertical, true): return CGSize(width: 0, height: half)
        case (.vertical, false): return CGSize(width: 0, height: -half)
        }
    }

    private func drag(_ translation: CGSize) {
        let start = dragStart ?? metrics.secondaryLength(for: CGFloat(secondaryLength))
        dragStart = start
        let along = axis == .horizontal ? translation.width : translation.height
        // Dragging toward the primary pane grows the secondary one.
        let delta = secondaryFirst ? along : -along
        secondaryLength = Double(metrics.clamped(start + delta))
    }
}

/// The divider's hit area and hairline. Dragging reports the cumulative translation; `onEnd`
/// lets the container forget where the drag started.
private struct SplitHandle: View {
    static let hitThickness: CGFloat = 9

    let axis: Axis
    let onDrag: (CGSize) -> Void
    let onEnd: () -> Void
    let onReset: () -> Void

    @State private var isHovering = false
    @State private var isDragging = false

    var body: some View {
        let horizontal = axis == .horizontal
        Rectangle()
            .fill(Color.clear)
            .frame(
                width: horizontal ? Self.hitThickness : nil, height: horizontal ? nil : Self.hitThickness
            )
            .overlay {
                Rectangle()
                    .fill(isHovering || isDragging ? Color.accentColor.opacity(0.6) : Color(nsColor: .separatorColor))
                    .frame(width: horizontal ? (isHovering || isDragging ? 2 : 1) : nil)
                    .frame(height: horizontal ? nil : (isHovering || isDragging ? 2 : 1))
            }
            .contentShape(Rectangle())
            .onHover { inside in
                guard inside != isHovering else { return }
                isHovering = inside
                if inside {
                    (horizontal ? NSCursor.resizeLeftRight : NSCursor.resizeUpDown).push()
                } else {
                    NSCursor.pop()
                }
            }
            .onDisappear {
                if isHovering {
                    NSCursor.pop()
                    isHovering = false
                }
            }
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged {
                        isDragging = true
                        onDrag($0.translation)
                    }
                    .onEnded { _ in
                        isDragging = false
                        onEnd()
                    }
            )
            .simultaneousGesture(TapGesture(count: 2).onEnded { onReset() })
            .accessibilityLabel("Resize panes")
            .accessibilityAddTraits(.isButton)
    }
}
