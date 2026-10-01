import Foundation

struct PineBoxOutput: Sendable, Identifiable, Equatable {
    let id: Int
    var left: Int
    var top: Double
    var right: Int
    var bottom: Double
    var borderColor: UInt32?
    var borderWidth: Int
    var backgroundColor: UInt32?
    /// Made with `xloc.bar_time`: setters take times, which are mapped to bar indexes.
    var timeAnchored = false
}
