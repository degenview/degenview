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
}
