import Foundation

enum PineQualifier: Int, Codable, Comparable, Sendable {
    case constant, input, simple, series
    static func < (lhs: PineQualifier, rhs: PineQualifier) -> Bool { lhs.rawValue < rhs.rawValue }
}
