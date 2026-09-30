import Foundation

struct PineSourcePosition: Codable, Equatable, Sendable {
    var line: Int
    var column: Int
    var offset: Int
}
