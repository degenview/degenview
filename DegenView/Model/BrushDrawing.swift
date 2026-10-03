import Foundation
import SwiftUI

/// Look of a brush stroke. Colours reuse the trend-line palette; width is in points.
struct BrushStyle: Codable, Equatable, Hashable {
    var color: TrendLineColor
    var lineWidth: Double
    var opacity: Double

    static let `default` = BrushStyle(color: .blue, lineWidth: 2, opacity: 1)

    static let widths: [Double] = [1, 2, 3, 4, 6, 8]
    static let opacities: [Double] = [0.25, 0.5, 0.75, 1]
}

/// A freehand stroke, pinned to the data rather than to pixels.
///
/// Every point is a `TrendAnchor` — continuous time plus price — never a screen point
/// or a candle index, so a stroke survives zoom, resize, scrolling and timeframe
/// switches. Time is deliberately *not* snapped to candle opens: a handwritten curve
/// needs sub-candle horizontal precision, and that precision lives in the drawing's
/// own coordinates, never in the canonical candles.
///
/// A click that never travelled is stored as a single point and drawn as a dot, so a
/// stroke always has at least one point.
struct BrushDrawing: Codable, Equatable, Hashable, Identifiable {
    static let schemaVersion = 1

    var schemaVersion = BrushDrawing.schemaVersion
    var id = UUID()
    var points: [TrendAnchor]
    var color: TrendLineColor
    var lineWidth: Double
    var opacity: Double
    var isLocked = false
    var createdAt = Date()
    var updatedAt = Date()

    init(points: [TrendAnchor], style: BrushStyle = .default) {
        self.points = points
        color = style.color
        lineWidth = style.lineWidth
        opacity = style.opacity
    }

    var style: BrushStyle {
        get { BrushStyle(color: color, lineWidth: lineWidth, opacity: opacity) }
        set {
            color = newValue.color
            lineWidth = newValue.lineWidth
            opacity = newValue.opacity
        }
    }
}

/// A stroke being drawn right now. Lives on the chart view model only; nothing is
/// written until the pointer is released.
struct BrushDraft: Equatable {
    var points: [TrendAnchor]
    var style: BrushStyle
}

/// Everything a chart view needs to draw brush strokes, bundled so the views take one
/// parameter.
struct BrushOverlayState {
    var strokes: [BrushDrawing] = []
    var draft: BrushDraft?
    var selectedID: UUID?
    var hoveredID: UUID?

    static let empty = BrushOverlayState()

    var isEmpty: Bool { strokes.isEmpty && draft == nil }
}

/// Capture and simplification tuning, in screen points. One place so the feel of the
/// brush can be adjusted without hunting.
enum BrushTuning {
    /// A drag sample is kept once the pointer has moved this far from the last kept one.
    static let sampleDistance: CGFloat = 2
    /// Ramer–Douglas–Peucker tolerance. Small enough that handwriting does not visibly
    /// change, large enough to drop the points on a straight run.
    static let simplifyTolerance: CGFloat = 0.75
    /// A stroke whose whole path is shorter than this is a dot.
    static let dotMaxTravel: CGFloat = 3
    /// A vertex that turns more than this is drawn as a corner, not smoothed away.
    static let cornerAngle: CGFloat = 70 * .pi / 180
    /// Safety cap on one live stroke.
    static let maxDraftPoints = 5000
    /// A move shorter than this is a click, not a drag.
    static let moveThreshold: CGFloat = 3
}
