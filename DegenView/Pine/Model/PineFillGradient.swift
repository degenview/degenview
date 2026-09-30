import Foundation

/// One bar of a gradient `fill()`: color runs from `topColor` at `top` to `bottomColor`
/// at `bottom`.
struct PineFillGradient: Sendable, Equatable {
    var top: Double
    var bottom: Double
    var topColor: UInt32
    var bottomColor: UInt32
}
