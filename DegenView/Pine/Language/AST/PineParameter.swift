import Foundation

struct PineParameter: Sendable {
    var name: String
    var type: PineValueType?
    var defaultValue: PineExpression?
}
