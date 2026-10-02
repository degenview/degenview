import Foundation

struct PineLabelOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var x: Int
    var y: Double
    var text: String
    var color: UInt32?
    var textColor: UInt32
    var style: PineLabelStyle
    var size: PineSize
    /// Hover text from `tooltip =` or `label.set_tooltip`. Kept with the label; the chart does not show it yet.
    var tooltip: String? = nil
    var isComplete: Bool { PineDrawingCoordinate.isKnown(x) && PineDrawingCoordinate.isKnown(y) }
    /// How the lines of a multi-line label align with each other.
    var textAlign: PineTextAlign = .center
    /// Made with `xloc.bar_time`: setters take times, which are mapped to bar indexes.
    var timeAnchored = false
}
