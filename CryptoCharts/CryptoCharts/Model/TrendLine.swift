import Foundation
import SwiftUI

enum TrendLineColor: String, Codable, CaseIterable, Hashable {
    case blue, green, red, orange, purple, gray

    var title: String { rawValue.capitalized }

    var color: Color {
        switch self {
        case .blue:   return .blue
        case .green:  return .green
        case .red:    return .red
        case .orange: return .orange
        case .purple: return .purple
        case .gray:   return .gray
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
        case .thin:       return "Thin"
        case .medium:     return "Medium"
        case .thick:      return "Thick"
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
/// the chart view model only, and the next click puts it away.
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
}

/// Which drawing tool the window's tool strip has armed.
enum ChartTool {
    case none
    case crosshair
    case trendLine
    case ruler
}
