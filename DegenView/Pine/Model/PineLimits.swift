import Foundation

struct PineLimits: Sendable {
    /// A guard against runaway input, not a Pine rule: published scripts reach 300,000 characters.
    var sourceCharacters = 500_000
    var tokens = 50_000
    var astNodes = 50_000
    var instructionsPerBar = 100_000
    var callDepth = 64
    var deadline: TimeInterval = 10
    static let `default` = PineLimits()
}
