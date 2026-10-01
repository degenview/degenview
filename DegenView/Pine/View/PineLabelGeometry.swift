import CoreGraphics

/// Where a label's text and shape sit relative to its anchor point, for each `label.style_*`. Pure geometry,
/// so the placement can be tested without drawing.
enum PineLabelGeometry {
    /// A glyph drawn at the anchor by the marker styles.
    enum Marker: Equatable {
        case cross, xcross, triangleUp, triangleDown, arrowUp, arrowDown, flag
    }

    enum Shape: Equatable {
        /// A rounded rectangle behind the text, with a pointer (three points ending at the anchor) or none.
        case bubble(pointer: [CGPoint])
        case ellipse
        case rectangle
        case diamond
        /// A glyph in `Layout.glyph`, with the text beside it.
        case marker(Marker)
        /// Text only: no bubble, no glyph.
        case textOnly
    }

    struct Layout: Equatable {
        /// The rectangle the text is centred in (and, for bubbles and shapes, the filled area).
        var body: CGRect
        var shape: Shape
        /// The glyph of a marker style.
        var glyph: CGRect?
    }

    /// `size` is the text plus padding; `pointer` the half-width and length of a bubble's pointer.
    static func layout(_ style: PineLabelStyle, anchor: CGPoint, size: CGSize, pointer p: CGFloat) -> Layout {
        func rect(x: CGFloat, y: CGFloat) -> CGRect { CGRect(origin: CGPoint(x: x, y: y), size: size) }
        func vertical(baseY: CGFloat) -> Shape {
            .bubble(pointer: [CGPoint(x: anchor.x - p, y: baseY), anchor, CGPoint(x: anchor.x + p, y: baseY)])
        }
        func horizontal(baseX: CGFloat) -> Shape {
            .bubble(pointer: [CGPoint(x: baseX, y: anchor.y - p), anchor, CGPoint(x: baseX, y: anchor.y + p)])
        }
        let centred = rect(x: anchor.x - size.width / 2, y: anchor.y - size.height / 2)
        switch style {
        case .labelDown:
            let body = rect(x: anchor.x - size.width / 2, y: anchor.y - p - size.height)
            return Layout(body: body, shape: vertical(baseY: body.maxY))
        case .labelUp:
            let body = rect(x: anchor.x - size.width / 2, y: anchor.y + p)
            return Layout(body: body, shape: vertical(baseY: body.minY))
        case .labelLeft:
            let body = rect(x: anchor.x + p, y: anchor.y - size.height / 2)
            return Layout(body: body, shape: horizontal(baseX: body.minX))
        case .labelRight:
            let body = rect(x: anchor.x - p - size.width, y: anchor.y - size.height / 2)
            return Layout(body: body, shape: horizontal(baseX: body.maxX))
        case .labelLowerLeft, .labelLowerRight, .labelUpperLeft, .labelUpperRight:
            return corner(style, anchor: anchor, size: size, pointer: p)
        case .labelCenter: return Layout(body: centred, shape: .bubble(pointer: []))
        case .none, .textOutline: return Layout(body: centred, shape: .textOnly)
        case .circle: return Layout(body: ellipseBody(centred), shape: .ellipse)
        case .square: return Layout(body: centred, shape: .rectangle)
        case .diamond:
            // The diamond's corners reach the edges of a rectangle twice as large as the text needs.
            return Layout(body: centred.insetBy(dx: -size.width / 2, dy: -size.height / 2), shape: .diamond)
        case .cross, .xcross, .flag, .triangledown, .arrowdown:
            return markerLayout(style, anchor: anchor, size: size, textAbove: true)
        case .triangleup, .arrowup:
            return markerLayout(style, anchor: anchor, size: size, textAbove: false)
        }
    }

    /// A circle that holds the text: its diameter is the text's diagonal.
    private static func ellipseBody(_ text: CGRect) -> CGRect {
        let diameter = (text.width * text.width + text.height * text.height).squareRoot()
        return CGRect(
            x: text.midX - diameter / 2, y: text.midY - diameter / 2, width: diameter, height: diameter)
    }

    /// The anchor is a corner of the bubble, with a pointer there.
    private static func corner(_ style: PineLabelStyle, anchor: CGPoint, size: CGSize, pointer p: CGFloat)
        -> Layout
    {
        let leftEdge = style == .labelLowerLeft || style == .labelUpperLeft
        let top = style == .labelUpperLeft || style == .labelUpperRight
        let origin = CGPoint(
            x: leftEdge ? anchor.x : anchor.x - size.width, y: top ? anchor.y : anchor.y - size.height)
        let horizontal = CGPoint(x: anchor.x + (leftEdge ? p : -p), y: anchor.y)
        let vertical = CGPoint(x: anchor.x, y: anchor.y + (top ? p : -p))
        return Layout(
            body: CGRect(origin: origin, size: size), shape: .bubble(pointer: [horizontal, anchor, vertical]))
    }

    /// A glyph centred on the anchor, as tall as the text line, with the text above or below it.
    private static func markerLayout(
        _ style: PineLabelStyle, anchor: CGPoint, size: CGSize, textAbove: Bool
    ) -> Layout {
        let side = size.height
        let glyph = CGRect(x: anchor.x - side / 2, y: anchor.y - side / 2, width: side, height: side)
        let originY = textAbove ? glyph.minY - size.height : glyph.maxY
        let body = CGRect(x: anchor.x - size.width / 2, y: originY, width: size.width, height: size.height)
        let marker: Marker =
            switch style {
            case .cross: .cross
            case .xcross: .xcross
            case .flag: .flag
            case .triangledown: .triangleDown
            case .triangleup: .triangleUp
            case .arrowdown: .arrowDown
            default: .arrowUp
            }
        return Layout(body: body, shape: .marker(marker), glyph: glyph)
    }
}
