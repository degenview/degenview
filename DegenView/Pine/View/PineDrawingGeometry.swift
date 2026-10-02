import SwiftUI

extension PineLineStyle {
    /// The dash pattern a stroke of this style uses; the arrow styles are solid lines with arrowheads.
    var dashPattern: [CGFloat] {
        switch self {
        case .dashed: [6, 4]
        case .dotted: [1, 3]
        case .solid, .arrowLeft, .arrowRight, .arrowBoth: []
        }
    }

    /// Whether the style puts an arrowhead at the line's first and second point.
    var arrowheads: (start: Bool, end: Bool) {
        switch self {
        case .arrowLeft: (true, false)
        case .arrowRight: (false, true)
        case .arrowBoth: (true, true)
        default: (false, false)
        }
    }
}

/// Pure geometry for the pieces of a script drawing that are more than a stroke or a fill.
enum PineDrawingGeometry {
    /// The triangle of an arrowhead at `tip` for a line arriving from `origin`.
    static func arrowhead(tip: CGPoint, from origin: CGPoint, length: CGFloat, halfWidth: CGFloat) -> [CGPoint] {
        let dx = tip.x - origin.x
        let dy = tip.y - origin.y
        let distance = (dx * dx + dy * dy).squareRoot()
        guard distance > 0 else { return [] }
        let ux = dx / distance
        let uy = dy / distance
        let base = CGPoint(x: tip.x - ux * length, y: tip.y - uy * length)
        return [
            tip, CGPoint(x: base.x - uy * halfWidth, y: base.y + ux * halfWidth),
            CGPoint(x: base.x + uy * halfWidth, y: base.y - ux * halfWidth),
        ]
    }

    /// Where a box's text is drawn and which part of the text sits on that point.
    static func boxTextPlacement(
        in rect: CGRect, horizontal: PineTextAlign, vertical: PineTextAlign, margin: CGFloat
    ) -> (point: CGPoint, anchor: UnitPoint) {
        let x: CGFloat
        let anchorX: CGFloat
        switch horizontal {
        case .left: (x, anchorX) = (rect.minX + margin, 0)
        case .right: (x, anchorX) = (rect.maxX - margin, 1)
        default: (x, anchorX) = (rect.midX, 0.5)
        }
        let y: CGFloat
        let anchorY: CGFloat
        switch vertical {
        case .top: (y, anchorY) = (rect.minY + margin, 0)
        case .bottom: (y, anchorY) = (rect.maxY - margin, 1)
        default: (y, anchorY) = (rect.midY, 0.5)
        }
        return (CGPoint(x: x, y: y), UnitPoint(x: anchorX, y: anchorY))
    }
}
