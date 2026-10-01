import Foundation

struct PineParameter: Sendable {
    var name: String
    var type: PineValueType?
    /// The script-defined type (or enum, or `chart.point`) the parameter was annotated with. `type` is
    /// `.object` for those; the name is what picks a method by its receiver.
    var typeName: String? = nil
    var defaultValue: PineExpression?
}
