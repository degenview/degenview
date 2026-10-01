import Foundation

/// Scope and structure rules that need no type information.
///
/// | Code | Rule |
/// |---|---|
/// | `PINE3020` | variable already declared in this scope |
/// | `PINE3021` | a `bool` initialised with `na` |
/// | `PINE3022` | `:=` on an undeclared variable (function bodies see only their parameters) |
/// | `PINE3023` | `break`/`continue` outside a loop |
/// | `PINE3024` | function declared twice |
/// | `PINE9003` | unsupported `request.*` call, anywhere in the script |
struct PineStructureValidator {
    private var diagnostics: [PineDiagnostic] = []

    static func validate(_ statements: [PineStatement]) -> [PineDiagnostic] {
        var validator = PineStructureValidator()
        validator.validate(statements, inherited: [], inLoop: false)
        PineStatement.forEachCall(in: statements) { name, range in
            guard name.hasPrefix("request."), name != "request.security" else { return }
            validator.report(
                "PINE9003", .unsupported, "Feature '\(name)' is not supported in this release.", range)
        }
        return validator.diagnostics
    }

    private mutating func validate(
        _ statements: [PineStatement], inherited: Set<String>, inLoop: Bool
    ) {
        var declared = inherited
        for statement in statements {
            switch statement {
            case .declaration(let name, let annotation, _, let value, let range):
                declare(name, &declared, range)
                if annotation.type == .bool, case .literal(.na, _) = value {
                    report("PINE3021", .semantic, "Boolean values cannot be na in Pine v6.", range)
                }
            case .assignment(let name, _, _, let range):
                if !declared.contains(name) {
                    report("PINE3022", .semantic, "Cannot reassign undeclared variable '\(name)'.", range)
                }
            case .tupleDeclaration(let names, _, let range):
                for name in names where name != "_" { declare(name, &declared, range) }
            case .loopControl(_, let range):
                if !inLoop {
                    report(
                        "PINE3023", .semantic, "break and continue are only allowed inside a loop.", range)
                }
            case .function(let name, let parameters, let body, let range):
                if declared.contains(name) {
                    report("PINE3024", .semantic, "Function '\(name)' is already declared.", range)
                }
                declared.insert(name)
                // Function bodies are their own scope: locals may reuse global names, and
                // globals cannot be reassigned from inside a function.
                validate(body, inherited: Set(parameters.map(\.name)), inLoop: false)
            case .conditional, .forRange, .forIn, .whileLoop, .switchStatement:
                validateNested(statement, declared: declared, inLoop: inLoop)
            case .expression, .typeDeclaration, .enumDeclaration, .fieldAssignment: break
            }
        }
    }

    private mutating func validateNested(
        _ statement: PineStatement, declared: Set<String>, inLoop: Bool
    ) {
        var inherited = declared
        var loop = inLoop
        switch statement {
        case .forRange(let variable, _, _, _, _, _):
            inherited.insert(variable)
            loop = true
        case .forIn(let index, let value, _, _, _):
            inherited.formUnion([index, value].compactMap { $0 })
            loop = true
        case .whileLoop: loop = true
        default: break
        }
        for block in statement.nestedBlocks { validate(block, inherited: inherited, inLoop: loop) }
    }

    private mutating func declare(_ name: String, _ declared: inout Set<String>, _ range: PineSourceRange) {
        if declared.contains(name) {
            report("PINE3020", .semantic, "Variable '\(name)' is already declared in this scope.", range)
        }
        declared.insert(name)
    }

    private mutating func report(
        _ code: String, _ category: PineDiagnosticCategory, _ message: String,
        _ range: PineSourceRange
    ) {
        diagnostics.append(.error(code, category, message, range))
    }
}
