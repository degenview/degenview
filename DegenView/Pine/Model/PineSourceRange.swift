import Foundation

struct PineSourceRange: Codable, Equatable, Sendable {
    var start: PineSourcePosition
    var end: PineSourcePosition
    static let zero = PineSourceRange(
        start: .init(line: 1, column: 1, offset: 0), end: .init(line: 1, column: 1, offset: 0))
}
