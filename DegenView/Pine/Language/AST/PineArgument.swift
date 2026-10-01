import Foundation

struct PineArgument: Sendable {
    var name: String?
    var value: PineExpression
}

extension Array where Element == PineArgument {
    /// The default value of an `input.*` call: `defval =`, or else the leading positional argument.
    /// A leading named argument such as `title =` is not the default.
    var inputDefault: PineArgument? {
        if let named = first(where: { $0.name == "defval" }) { return named }
        return first.flatMap { $0.name == nil ? $0 : nil }
    }
}
