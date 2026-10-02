import Foundation

/// Drawing objects anchor x coordinates to absolute `bar_index` values.
struct PineLineOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var x1: Int
    var y1: Double
    var x2: Int
    var y2: Double
    var color: UInt32
    var width: Int
    var style: PineLineStyle
    var extend: PineLineExtend
    /// Whether all four coordinates are known; a line missing one is not drawn.
    var isComplete: Bool {
        PineDrawingCoordinate.isKnown(x1) && PineDrawingCoordinate.isKnown(x2)
            && PineDrawingCoordinate.isKnown(y1) && PineDrawingCoordinate.isKnown(y2)
    }
    /// Made with `xloc.bar_time`: setters take times, which are mapped to bar indexes.
    var timeAnchored = false
}
