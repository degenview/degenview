import SwiftUI

extension ChartPlot {
    /// Freehand strokes, plus the one being drawn right now.
    ///
    /// Immediate-mode, like the other drawings: each stroke is projected once, smoothed into
    /// one `Path` and stroked once. Call inside the chart's clipped layer so a stroke
    /// anchored off-screen never reaches the gutter. Opacity is applied to the single
    /// stroke, so a stroke crossing itself does not darken.
    func drawBrushes(
        _ context: inout GraphicsContext,
        overlay: BrushOverlayState,
        points: [KlineData]
    ) {
        guard !points.isEmpty, !overlay.isEmpty else { return }
        let slot = slotWidth(forCount: points.count)

        func project(_ anchors: [TrendAnchor]) -> [CGPoint] {
            anchors.map { position(of: $0, points: points, slotWidth: slot) }
        }

        for stroke in overlay.strokes {
            let screen = project(stroke.points)
            let emphasis: Emphasis =
                stroke.id == overlay.selectedID ? .selected : stroke.id == overlay.hoveredID ? .hovered : .none
            drawBrush(
                &context, screen: screen, color: stroke.color.color,
                width: CGFloat(stroke.lineWidth), opacity: stroke.opacity, emphasis: emphasis)
        }

        if let draft = overlay.draft {
            drawBrush(
                &context, screen: project(draft.points), color: draft.style.color.color,
                width: CGFloat(draft.style.lineWidth), opacity: draft.style.opacity, emphasis: .none)
        }
    }

    private enum Emphasis {
        case none, hovered, selected
    }

    private func drawBrush(
        _ context: inout GraphicsContext,
        screen: [CGPoint],
        color: Color,
        width: CGFloat,
        opacity: Double,
        emphasis: Emphasis
    ) {
        guard let first = screen.first else { return }
        let isDot = screen.count == 1

        func shape() -> Path {
            isDot
                ? Path(ellipseIn: CGRect(x: first.x - width / 2, y: first.y - width / 2, width: width, height: width))
                : BrushGeometry.smoothedPath(screen)
        }

        if emphasis != .none {
            let halo = shape()
            let haloOpacity = emphasis == .selected ? 0.28 : 0.16
            if isDot {
                context.stroke(halo, with: .color(color.opacity(haloOpacity)), lineWidth: 6)
            } else {
                context.stroke(
                    halo, with: .color(color.opacity(haloOpacity)),
                    style: StrokeStyle(lineWidth: width + 6, lineCap: .round, lineJoin: .round))
            }
        }

        if isDot {
            context.fill(shape(), with: .color(color.opacity(opacity)))
        } else {
            context.stroke(
                shape(), with: .color(color.opacity(opacity)),
                style: StrokeStyle(lineWidth: width, lineCap: .round, lineJoin: .round))
        }

        if emphasis == .selected, let bounds = BrushGeometry.bounds(of: screen) {
            let box = bounds.insetBy(dx: -(width / 2 + 5), dy: -(width / 2 + 5))
            context.stroke(
                Path(roundedRect: box, cornerRadius: 3),
                with: .color(style.trendLineSelectedColor.opacity(0.8)),
                style: StrokeStyle(lineWidth: 1, dash: style.trendDashPattern))
        }
    }
}
