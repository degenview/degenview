import Foundation

/// One callable form of a Pine builtin: `ta.rsi(source, length) → series float`.
///
/// Overloads are separate signatures under the same name. Editor tooling (completion rows,
/// signature help) reads this; nothing in the compiler or runtime does.
struct PineSymbolSignature: Equatable, Sendable {
    struct Parameter: Equatable, Sendable {
        let name: String
        let type: String
        let defaultValue: String?
        let isOptional: Bool

        /// `source: series float` or `length: simple int = 14`.
        var detail: String {
            var text = "\(name): \(type)"
            if let defaultValue { text += " = \(defaultValue)" }
            return text
        }
    }

    /// The qualified name: `ta.rsi`, `plot`.
    let name: String
    let parameters: [Parameter]
    /// Pine's return type text, `series float`, `[series float, series float]` or `void`.
    let returns: String
    let summary: String?

    /// `ta.rsi(source, length) → series float`.
    var label: String {
        "\(name)(\(parameters.map(\.name).joined(separator: ", "))) → \(returns)"
    }

    /// The parameter list alone, `source, length`, for rows that already show the name.
    var parameterNames: String { parameters.map(\.name).joined(separator: ", ") }

    /// How many leading parameters a call must supply.
    var requiredCount: Int { parameters.prefix { !$0.isOptional }.count }
}
