import Foundation

struct PineLimits: Sendable {
    var sourceCharacters = 100_000
    var tokens = 50_000
    var astNodes = 50_000
    var instructionsPerBar = 100_000
    var callDepth = 64
    var deadline: TimeInterval = 10
    static let `default` = PineLimits()
}
