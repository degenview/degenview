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
}
