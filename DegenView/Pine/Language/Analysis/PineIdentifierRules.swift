import Foundation

/// What a user may name a variable, function, parameter or loop counter.
///
/// Pine identifiers are `[A-Za-z_][A-Za-z0-9_]*`, so a dotted name is never one. A name that a
/// builtin already owns cannot be reused. A bare namespace (`math`, `ta`, `color`) is not a
/// builtin variable or function, so it stays free: `math = 44` is fine while `math.max = 44` is not.
///
/// | Code | Rule |
/// |---|---|
/// | `CE10090` | user variable identifier contains `.` |
/// | `CE10190` | name shadows a builtin variable or function |
enum PineIdentifierRules {
    struct Violation: Equatable {
        let code: String
        let message: String
    }

    /// Builtin variables, and builtin functions. The object types (`line`, `label`, `box`,
    /// `table`) are in the function table as constructors but are namespaces in Pine, so they
    /// stay usable as plain names. `plot` and `input` are callable and also namespaces; they
    /// are functions first, so they are protected.
    private static let builtinVariables = PineSymbolCatalog.variables
    private static let builtinFunctions = PineSymbolCatalog.functions.subtracting(
        PineSymbolCatalog.objectTypes)

    static func violation(for name: String) -> Violation? {
        if name == "_" { return nil }
        if name.contains(".") {
            return Violation(
                code: "CE10090",
                message: "User variable identifiers should not contain \".\" character: \"\(name)\".")
        }
        if builtinVariables.contains(name) {
            return Violation(
                code: "CE10190", message: "Cannot shadow the built-in variable \"\(name)\".")
        }
        if builtinFunctions.contains(name) {
            return Violation(
                code: "CE10190", message: "Cannot shadow the built-in function \"\(name)\".")
        }
        return nil
    }
}
