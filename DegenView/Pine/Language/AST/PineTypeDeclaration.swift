import Foundation

/// One field of a script-defined `type`: `[type] name [= default]`.
struct PineTypeField: Sendable {
    var name: String
    var type: PineValueType?
    /// Evaluated at construction when `T.new()` does not supply the field; `na` when absent.
    var defaultValue: PineExpression?
}
