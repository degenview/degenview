import SwiftUI

/// `box.*`, `line.*` and `label.*` objects, positioned by absolute `bar_index`.
extension PineChartLayer {
    private static let labelPointer: CGFloat = 5
    private static let labelPadding = CGSize(width: 10, height: 6)

    func drawBoxes(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        for box in pine.boxes where box.isComplete {
            var left = x(forBar: box.left, plot: plot, slot: slot)
            var right = x(forBar: box.right, plot: plot, slot: slot)
            if box.extend.extendsLeft { left = min(left, right) - plot.plotRect.width * 2 }
            if box.extend.extendsRight { right = max(left, right) + plot.plotRect.width * 2 }
            let top = plot.y(for: box.top)
            let bottom = plot.y(for: box.bottom)
            let rect = CGRect(
                x: min(left, right), y: min(top, bottom), width: abs(right - left), height: abs(bottom - top))
            if let fill = box.backgroundColor {
                context.fill(Path(rect), with: .color(Color(pineRGBA: fill)))
            }
            if let border = box.borderColor, box.borderWidth > 0 {
                context.stroke(
                    Path(rect), with: .color(Color(pineRGBA: border)),
                    style: StrokeStyle(lineWidth: CGFloat(box.borderWidth), dash: box.borderStyle.dashPattern))
            }
            if !box.text.isEmpty, box.textColor & 0xFF != 0 {
                let text = context.resolve(
                    Text(box.text)
                        .font(.system(size: box.textSize.fontSize))
                        .foregroundColor(Color(pineRGBA: box.textColor)))
                let placement = PineDrawingGeometry.boxTextPlacement(
                    in: rect, horizontal: box.textHorizontalAlign, vertical: box.textVerticalAlign, margin: 4)
                context.drawLayer { layer in
                    layer.clip(to: Path(rect))
                    layer.draw(text, at: placement.point, anchor: placement.anchor)
                }
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
            guard let first = lines[fill.line1], let second = lines[fill.line2], first.isComplete,
                second.isComplete
            else { continue }
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

    /// Straight segments through each polyline's points: filled when it is closed and has a fill color,
    /// stroked in its line color. Under the lines, over the linefills.
    func drawPolylines(context: inout GraphicsContext, plot: ChartPlot) {
        guard !pine.polylines.isEmpty else { return }
        let slot = slotWidth(plot)
        for polyline in pine.polylines where polyline.points.count > 1 {
            let locations = polyline.points.map {
                CGPoint(x: x(forBar: $0.index, plot: plot, slot: slot), y: plot.y(for: $0.price))
            }
            let path = PineDrawingGeometry.polylinePath(
                through: locations, curved: polyline.curved, closed: polyline.closed)
            if polyline.closed, let fill = polyline.fillColor {
                context.fill(path, with: .color(Color(pineRGBA: fill)))
            }
            guard let color = polyline.lineColor, polyline.width > 0 else { continue }
            context.stroke(
                path, with: .color(Color(pineRGBA: color)),
                style: StrokeStyle(
                    lineWidth: CGFloat(polyline.width), lineCap: .round, lineJoin: .round,
                    dash: polyline.style.dashPattern))
        }
    }

    func drawLines(context: inout GraphicsContext, plot: ChartPlot) {
        let slot = slotWidth(plot)
        for line in pine.lines where line.isComplete {
            let start = CGPoint(x: x(forBar: line.x1, plot: plot, slot: slot), y: plot.y(for: line.y1))
            let end = CGPoint(x: x(forBar: line.x2, plot: plot, slot: slot), y: plot.y(for: line.y2))
            let (from, to) = extended(start, end, by: line.extend, in: plot.plotRect)
            var path = Path()
            path.move(to: from)
            path.addLine(to: to)
            context.stroke(
                path, with: .color(Color(pineRGBA: line.color)),
                style: StrokeStyle(
                    lineWidth: CGFloat(max(1, line.width)), lineCap: .round, dash: line.style.dashPattern))
            let heads = line.style.arrowheads
            let size = CGFloat(max(1, line.width)) * 2 + 6
            for (show, tip, origin) in [(heads.start, start, end), (heads.end, end, start)] where show {
                let points = PineDrawingGeometry.arrowhead(
                    tip: tip, from: origin, length: size, halfWidth: size / 2.5)
                guard points.count == 3 else { continue }
                var head = Path()
                head.move(to: points[0])
                head.addLine(to: points[1])
                head.addLine(to: points[2])
                head.closeSubpath()
                context.fill(head, with: .color(Color(pineRGBA: line.color)))
            }
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
        for label in pine.labels where label.isComplete {
            let anchor = CGPoint(x: x(forBar: label.x, plot: plot, slot: slot), y: plot.y(for: label.y))
            let text = context.resolve(
                Text(label.text)
                    .font(.system(size: label.size.fontSize))
                    .foregroundColor(Color(pineRGBA: label.textColor)))
            let measured = text.measure(in: CGSize(width: 600, height: 400))
            let size = CGSize(
                width: measured.width + Self.labelPadding.width,
                height: measured.height + Self.labelPadding.height)
            let layout = PineLabelGeometry.layout(
                label.style, anchor: anchor, size: size, pointer: Self.labelPointer)
            if let fill = label.color, fill & 0xFF != 0 {
                drawLabelShape(layout, fill: Color(pineRGBA: fill), in: &context)
            }
            context.draw(text, at: CGPoint(x: layout.body.midX, y: layout.body.midY))
        }
    }

    private func drawLabelShape(
        _ layout: PineLabelGeometry.Layout, fill: Color, in context: inout GraphicsContext
    ) {
        switch layout.shape {
        case .bubble(let pointer):
            var shape = Path(roundedRect: layout.body, cornerRadius: 3)
            if !pointer.isEmpty {
                shape.move(to: pointer[0])
                shape.addLine(to: pointer[1])
                shape.addLine(to: pointer[2])
                shape.closeSubpath()
            }
            context.fill(shape, with: .color(fill))
        case .ellipse: context.fill(Path(ellipseIn: layout.body), with: .color(fill))
        case .rectangle: context.fill(Path(roundedRect: layout.body, cornerRadius: 2), with: .color(fill))
        case .diamond:
            let box = layout.body
            var shape = Path()
            shape.move(to: CGPoint(x: box.midX, y: box.minY))
            shape.addLine(to: CGPoint(x: box.maxX, y: box.midY))
            shape.addLine(to: CGPoint(x: box.midX, y: box.maxY))
            shape.addLine(to: CGPoint(x: box.minX, y: box.midY))
            shape.closeSubpath()
            context.fill(shape, with: .color(fill))
        case .marker(let marker):
            guard let glyph = layout.glyph else { return }
            drawMarker(marker, in: glyph, fill: fill, context: &context)
        case .textOnly: break
        }
    }

    /// Simplified glyphs for the marker label styles.
    private func drawMarker(
        _ marker: PineLabelGeometry.Marker, in box: CGRect, fill: Color, context: inout GraphicsContext
    ) {
        var path = Path()
        switch marker {
        case .triangleUp, .arrowUp:
            path.move(to: CGPoint(x: box.midX, y: box.minY))
            path.addLine(to: CGPoint(x: box.maxX, y: box.maxY))
            path.addLine(to: CGPoint(x: box.minX, y: box.maxY))
            path.closeSubpath()
        case .triangleDown, .arrowDown:
            path.move(to: CGPoint(x: box.midX, y: box.maxY))
            path.addLine(to: CGPoint(x: box.maxX, y: box.minY))
            path.addLine(to: CGPoint(x: box.minX, y: box.minY))
            path.closeSubpath()
        case .flag:
            path.move(to: CGPoint(x: box.minX, y: box.maxY))
            path.addLine(to: CGPoint(x: box.minX, y: box.minY))
            path.addLine(to: CGPoint(x: box.maxX, y: box.minY + box.height / 3))
            path.addLine(to: CGPoint(x: box.minX, y: box.midY))
            context.stroke(path, with: .color(fill), lineWidth: 2)
            return
        case .cross:
            path.move(to: CGPoint(x: box.midX, y: box.minY))
            path.addLine(to: CGPoint(x: box.midX, y: box.maxY))
            path.move(to: CGPoint(x: box.minX, y: box.midY))
            path.addLine(to: CGPoint(x: box.maxX, y: box.midY))
            context.stroke(path, with: .color(fill), lineWidth: 2)
            return
        case .xcross:
            path.move(to: CGPoint(x: box.minX, y: box.minY))
            path.addLine(to: CGPoint(x: box.maxX, y: box.maxY))
            path.move(to: CGPoint(x: box.maxX, y: box.minY))
            path.addLine(to: CGPoint(x: box.minX, y: box.maxY))
            context.stroke(path, with: .color(fill), lineWidth: 2)
            return
        }
        context.fill(path, with: .color(fill))
    }
}
