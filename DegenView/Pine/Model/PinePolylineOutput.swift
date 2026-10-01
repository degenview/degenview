import Foundation

/// A `polyline`: straight segments through points anchored to bar index and price, optionally closed
/// and filled. `curved` is not drawn: segments are always straight.
struct PinePolylineOutput: Sendable, Identifiable, Equatable {
    struct Point: Sendable, Equatable {
        var index: Int
        var price: Double
    }

    let id: Int
    var points: [Point]
    var closed: Bool
    var lineColor: UInt32?
    var fillColor: UInt32?
    var style: PineLineStyle
    var width: Int
}
