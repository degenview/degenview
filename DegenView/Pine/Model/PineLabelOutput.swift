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
}
