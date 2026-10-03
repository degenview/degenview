import Foundation
import SwiftUI

enum TrendLineColor: String, Codable, CaseIterable, Hashable {
    case blue, green, red, orange, purple, gray

    var title: String { rawValue.capitalized }

    var color: Color {
        switch self {
        case .blue: return .blue
        case .green: return .green
        case .red: return .red
        case .orange: return .orange
        case .purple: return .purple
        case .gray: return .gray
        }
    }
}

enum TrendLineThickness: Double, Codable, CaseIterable, Hashable {
    case thin = 1
    case medium = 1.5
    case thick = 2.5
    case extraThick = 4

    var title: String {
        switch self {
        case .thin: return "Thin"
        case .medium: return "Medium"
        case .thick: return "Thick"
        case .extraThick: return "Extra Thick"
        }
    }
}

/// One end of a hand-drawn trend line, pinned to the data rather than to pixels.
///
/// Stored as time + price, never as a point index: the X axis is index space over
/// `visibleKlines`, which shifts on every zoom, timeframe switch and refetch. A date
/// survives all three, so the line stays on the price points it was drawn against.
struct TrendAnchor: Codable, Equatable, Hashable {
    var date: Date
    var price: Double
}

/// A straight line drawn on a chart between two anchors.
///
/// Shown on every timeframe, always projected from its anchors' true times. The
/// timeframes span very different windows — 1H covers two days, 1D covers two
/// months — so the same line is 30× narrower on 1D than on 1H, down to a few
/// pixels for a line drawn across a few hours. That is deliberate: the line marks
/// when it marks, and reads full-width in the other direction, where a line drawn
/// across weeks on 1D spans the whole 1H chart.
struct TrendLine: Codable, Equatable, Hashable, Identifiable {
    var id = UUID()
    var start: TrendAnchor
    var end: TrendAnchor
    /// Optional so lines written by older app versions decode with today's defaults.
    var color: TrendLineColor? = nil
    var thickness: TrendLineThickness? = nil

    var resolvedColor: TrendLineColor { color ?? .blue }
    var resolvedThickness: TrendLineThickness { thickness ?? .medium }
}

/// A measuring rectangle, pinned by two opposite corners.
///
/// Deliberately not `Codable`: a ruler is a measurement, not an annotation. It lives on
/// the chart view model only, is never persisted and never enters drawing history.
///
/// Which way the move went is read from the anchors rather than stored, so a rectangle
/// keeps its colour when a timeframe switch reprojects it.
struct RulerRect: Equatable, Identifiable {
    var id = UUID()
    var start: TrendAnchor
    var end: TrendAnchor

    /// True when the second corner landed above the first — measured bottom to top.
    /// A flat measurement counts as up, matching how the card header reads a 0% change.
    var isUpward: Bool { end.price >= start.price }

    var earliest: Date { min(start.date, end.date) }
    var latest: Date { max(start.date, end.date) }
    var high: Double { max(start.price, end.price) }
    var low: Double { min(start.price, end.price) }

    /// The anchor-space position of one of the four corners.
    func anchor(of corner: RulerCorner) -> TrendAnchor {
        TrendAnchor(
            date: corner.isLeft ? earliest : latest,
            price: corner.isTop ? high : low
        )
    }

    /// The rectangle with `corner` dragged to `anchor` and the opposite corner left
    /// where it was.
    ///
    /// Each axis is written to whichever stored anchor owns that edge, so the start and
    /// end — and with them the measured direction — keep their meaning. Dragging a corner
    /// past the opposite one flips the box without a special case.
    func resized(corner: RulerCorner, to anchor: TrendAnchor) -> RulerRect {
        var result = self
        let startIsEarlier = start.date <= end.date
        if corner.isLeft == startIsEarlier {
            result.start.date = anchor.date
        } else {
            result.end.date = anchor.date
        }
        let startIsHigher = start.price >= end.price
        if corner.isTop == startIsHigher {
            result.start.price = anchor.price
        } else {
            result.end.price = anchor.price
        }
        return result
    }

    /// The same rectangle shifted in time and price.
    func translated(by time: TimeInterval, price: Double) -> RulerRect {
        var result = self
        result.start = TrendAnchor(date: start.date.addingTimeInterval(time), price: start.price + price)
        result.end = TrendAnchor(date: end.date.addingTimeInterval(time), price: end.price + price)
        return result
    }
}

/// A corner of a ruler rectangle. "Left" is the earlier time, "top" the higher price —
/// named for how the box reads on screen, whichever way it was drawn.
enum RulerCorner: CaseIterable, Equatable {
    case topLeft, topRight, bottomLeft, bottomRight

    var isLeft: Bool { self == .topLeft || self == .bottomLeft }
    var isTop: Bool { self == .topLeft || self == .topRight }
}

/// What the pointer is over on a ruler: a corner resizes it, an edge moves it. The
/// interior is deliberately not a target, so a new measurement can start inside an old one.
enum RulerPart: Equatable {
    case corner(RulerCorner)
    case edge
}

struct RulerHit: Equatable {
    var id: UUID
    var part: RulerPart
}

/// Everything a chart view needs to draw its rulers, bundled so the views take one
/// parameter instead of four.
struct RulerOverlayState {
    var rects: [RulerRect] = []
    /// The rectangle being drawn right now.
    var draft: (start: TrendAnchor, end: TrendAnchor)?
    var selectedID: UUID?
    var hover: RulerHit?

    static let empty = RulerOverlayState()

    var isEmpty: Bool { rects.isEmpty && draft == nil }
}

/// Which drawing tool the window's tool strip has armed.
enum ChartTool {
    case none
    case crosshair
    case trendLine
    case fibonacciRetracement
    case brush
    case ruler
}
