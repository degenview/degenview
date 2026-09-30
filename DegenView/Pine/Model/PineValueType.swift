import Foundation

enum PineValueType: String, Codable, Hashable, Sendable {
    case int, float, bool, string, color
    case line, label, box, table, array
    /// `input.time`: a millisecond timestamp carried as an `int`.
    case time
}
