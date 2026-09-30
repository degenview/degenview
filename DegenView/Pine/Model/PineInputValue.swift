import Foundation

enum PineInputValue: Codable, Equatable, Hashable, Sendable {
    case int(Int)
    case float(Double)
    case bool(Bool)
    case string(String)
    case color(UInt32)
    case source(String)
}
