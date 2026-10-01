import Foundation

struct PineLimits: Sendable {
    /// A guard against runaway input, not a Pine rule: published scripts reach 300,000 characters.
    var sourceCharacters = 500_000
    var tokens = 50_000
    var astNodes = 50_000
    /// What one bar may execute. Pine limits a bar by time, not by steps: a volume-profile script that
    /// rebuilds its profile on the last bar needs several million. The 10 s deadline still bounds the run.
    var instructionsPerBar = 20_000_000
    var callDepth = 64
    var deadline: TimeInterval = 10
    static let `default` = PineLimits()
}
