import Foundation

enum PineValueType: String, Codable, Hashable, Sendable {
    case int, float, bool, string, color
    case line, label, box, table, array
    /// An instance of a script-defined `type`. Runtime values are dynamic, so which type is not tracked.
    case object
    /// `map<K, V>`; key and value types are not tracked.
    case map
    /// `matrix<T>`; the element type is not tracked.
    case matrix
    /// `input.time`: a millisecond timestamp carried as an `int`.
    case time
}
