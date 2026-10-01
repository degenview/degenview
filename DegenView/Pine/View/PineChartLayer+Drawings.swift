import SwiftUI

/// `box.*`, `line.*` and `label.*` objects, positioned by absolute `bar_index`.
extension PineChartLayer {
    private static let labelPointer: CGFloat = 5
    private static let labelPadding = CGSize(width: 10, height: 6)

    func drawBoxes(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        for box in pine.boxes {
            let left = x(forBar: box.left, plot: plot, slot: slot)
            let right = x(forBar: box.right, plot: plot, slot: slot)
            let top = plot.y(for: box.top)
            let bottom = plot.y(for: box.bottom)
            let rect = CGRect(
                x: min(left, right), y: min(top, bottom), width: abs(right - left), height: abs(bottom - top))
            if let fill = box.backgroundColor {
                context.fill(Path(rect), with: .color(Color(pineRGBA: fill)))
            }
            if let border = box.borderColor, box.borderWidth > 0 {
                context.stroke(
                    Path(rect), with: .color(Color(pineRGBA: border)), lineWidth: CGFloat(box.borderWidth))
            }
        }
    }

    /// The quadrilateral between a linefill's two lines, under the lines themselves.
    func drawLinefills(context: inout GraphicsContext, plot: ChartPlot) {
        guard !pine.linefills.isEmpty else { return }
        let slot = slotWidth(plot)
        let lines = Dictionary(uniqueKeysWithValues: pine.lines.map { ($0.id, $0) })
        func ends(_ line: PineLineOutput) -> (CGPoint, CGPoint) {
            (
                CGPoint(x: x(forBar: line.x1, plot: plot, slot: slot), y: plot.y(for: line.y1)),
                CGPoint(x: x(forBar: line.x2, plot: plot, slot: slot), y: plot.y(for: line.y2))
            )
        }
        for fill in pine.linefills {
            guard let first = lines[fill.line1], let second = lines[fill.line2] else { continue }
            let (a, b) = ends(first)
            let (c, d) = ends(second)
            var path = Path()
            path.move(to: a)
            path.addLine(to: b)
            path.addLine(to: d)
            path.addLine(to: c)
            path.closeSubpath()
            context.fill(path, with: .color(Color(pineRGBA: fill.color)))
        }
    }

    func drawLines(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        for line in pine.lines {
            let start = CGPoint(x: x(forBar: line.x1, plot: plot, slot: slot), y: plot.y(for: line.y1))
            let end = CGPoint(x: x(forBar: line.x2, plot: plot, slot: slot), y: plot.y(for: line.y2))
            let (from, to) = extended(start, end, by: line.extend, in: plot.plotRect)
            var path = Path()
            path.move(to: from)
            path.addLine(to: to)
            let dash: [CGFloat] =
                switch line.style {
                case .dashed: [6, 4]
                case .dotted: [1, 3]
                default: []
                }
            context.stroke(
                path, with: .color(Color(pineRGBA: line.color)),
                style: StrokeStyle(lineWidth: CGFloat(max(1, line.width)), lineCap: .round, dash: dash))
        }
    }

    /// The segment's endpoints, pushed out to the plot's edges along its slope when the line
    /// extends left and/or right. Vertical lines never extend.
    private func extended(
        _ a: CGPoint, _ b: CGPoint, by extend: PineLineExtend, in rect: CGRect
    ) -> (CGPoint, CGPoint) {
        guard a.x != b.x else { return (a, b) }
        let slope = (b.y - a.y) / (b.x - a.x)
        var (left, right) = a.x < b.x ? (a, b) : (b, a)
        if extend.extendsLeft {
            left = CGPoint(x: rect.minX, y: left.y - slope * (left.x - rect.minX))
        }
        if extend.extendsRight {
            right = CGPoint(x: rect.maxX, y: right.y + slope * (rect.maxX - right.x))
        }
        return (left, right)
    }

    // MARK: - Labels

    /// Labels draw as a bubble with a pointer toward their anchor. `label_down` sits
    /// above the anchor, `label_up` below it; a transparent label color leaves just text.
    func drawLabels(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        for label in pine.labels {
            let anchor = CGPoint(x: x(forBar: label.x, plot: plot, slot: slot), y: plot.y(for: label.y))
            let text = context.resolve(
                Text(label.text)
                    .font(.system(size: label.size.fontSize))
                    .foregroundColor(Color(pineRGBA: label.textColor)))
            let measured = text.measure(in: CGSize(width: 600, height: 400))
            let size = CGSize(
                width: measured.width + Self.labelPadding.width,
                height: measured.height + Self.labelPadding.height)
            let (bubble, tip) = labelGeometry(label.style, anchor: anchor, size: size)
            if let fill = label.color, fill & 0xFF != 0, label.style != .none {
                var shape = Path(roundedRect: bubble, cornerRadius: 3)
                if !tip.isEmpty {
                    shape.move(to: tip[0])
                    shape.addLine(to: tip[1])
                    shape.addLine(to: tip[2])
                    shape.closeSubpath()
                }
                context.fill(shape, with: .color(Color(pineRGBA: fill)))
            }
            context.draw(text, at: CGPoint(x: bubble.midX, y: bubble.midY))
        }
    }

    /// The bubble rectangle for a label of `size` anchored at `anchor`, and the three points of
    /// its pointer (empty when it has none).
    private func labelGeometry(
        _ style: PineLabelStyle, anchor: CGPoint, size: CGSize
    ) -> (bubble: CGRect, tip: [CGPoint]) {
        let p = Self.labelPointer
        var bubble = CGRect(origin: .zero, size: size)
        func verticalTip(baseY: CGFloat) -> [CGPoint] {
            [CGPoint(x: anchor.x - p, y: baseY), anchor, CGPoint(x: anchor.x + p, y: baseY)]
        }
        func horizontalTip(baseX: CGFloat) -> [CGPoint] {
            [CGPoint(x: baseX, y: anchor.y - p), anchor, CGPoint(x: baseX, y: anchor.y + p)]
        }
        switch style {
        case .labelDown:
            bubble.origin = CGPoint(x: anchor.x - size.width / 2, y: anchor.y - p - size.height)
            return (bubble, verticalTip(baseY: bubble.maxY))
        case .labelUp:
            bubble.origin = CGPoint(x: anchor.x - size.width / 2, y: anchor.y + p)
            return (bubble, verticalTip(baseY: bubble.minY))
        case .labelLeft:
            bubble.origin = CGPoint(x: anchor.x + p, y: anchor.y - size.height / 2)
            return (bubble, horizontalTip(baseX: bubble.minX))
        case .labelRight:
            bubble.origin = CGPoint(x: anchor.x - p - size.width, y: anchor.y - size.height / 2)
            return (bubble, horizontalTip(baseX: bubble.maxX))
        case .labelCenter, .none:
            bubble.origin = CGPoint(x: anchor.x - size.width / 2, y: anchor.y - size.height / 2)
            return (bubble, [])
        }
    }
}
