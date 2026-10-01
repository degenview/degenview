import Foundation

/// A `linefill`: the area between two `line` objects, painted in one color.
struct PineLinefillOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var line1: Int
    var line2: Int
    var color: UInt32
}
