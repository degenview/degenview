import Foundation

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
}

/// Which drawing tool the window's tool strip has armed.
enum ChartTool {
    case none
    case trendLine
}
