import Foundation

/// Area between two plots; `colors` is per bar, parallel to the plots' values.
struct PineFillOutput: Sendable, Identifiable {
    let id: Int
    var plotA: Int
    var plotB: Int
    var colors: [UInt32?]
    /// Per-bar gradients for the `fill(p1, p2, top_value, bottom_value, top_color,
    /// bottom_color)` overload; parallel to `colors` and empty for flat fills.
    var gradients: [PineFillGradient?] = []
}
