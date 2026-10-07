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
/// | `CE10090`, `CE10190` | a declared name is dotted or shadows a builtin (`PineIdentifierRules`) |
/// | `PINE3048` | a bare literal, name or operator expression as a statement (`3223`, `"x"`) |
/// | `PINE9003` | unsupported `request.*` call, anywhere in the script |
struct PineStructureValidator {
    private var diagnostics: [PineDiagnostic] = []
    private var source = ""

    /// What picks one definition of a method: the type of its first (receiver) parameter.
    private struct ReceiverSignature: Hashable {
        var type: PineValueType?
        var typeName: String?
    }

    /// Names declared with `method`: unlike functions, they may be defined again for another receiver type.
    private var methods: Set<String> = []
    private var methodReceivers: [String: Set<ReceiverSignature>] = [:]

    static func validate(
        _ statements: [PineStatement], methods: Set<String> = [], source: String = ""
    ) -> [PineDiagnostic] {
        var validator = PineStructureValidator()
        validator.methods = methods
        validator.source = source
        validator.validate(statements, inherited: [], inLoop: false, isNested: false)
        PineStatement.forEachCall(in: statements) { name, range in
            guard name.hasPrefix("request."), name != "request.security",
                name != "request.security_lower_tf"
            else { return }
            validator.report(
                "PINE9003", .unsupported, "Feature '\(name)' is not supported in this release.", range)
        }
        return validator.diagnostics
    }

    private mutating func validate(
        _ statements: [PineStatement], inherited: Set<String>, inLoop: Bool, isNested: Bool
    ) {
        var declared = inherited
        for (index, statement) in statements.enumerated() {
            switch statement {
            case .declaration(let name, let annotation, _, let value, let range):
                declare(name, &declared, range)
                checkName(name, range)
                if annotation.type == .bool, case .literal(.na, _) = value {
                    report("PINE3021", .semantic, "Boolean values cannot be na in Pine v6.", range)
                }
            case .assignment(let name, _, _, let range):
                if !declared.contains(name) {
                    report("PINE3022", .semantic, "Cannot reassign undeclared variable '\(name)'.", range)
                }
            case .tupleDeclaration(let names, _, let range):
                for name in names where name != "_" {
                    declare(name, &declared, range)
                    checkName(name, range)
                }
            case .loopControl(_, let range):
                if !inLoop {
                    report(
                        "PINE3023", .semantic, "break and continue are only allowed inside a loop.", range)
                }
            case .function(let name, let parameters, let body, let range):
                let receiver = ReceiverSignature(
                    type: parameters.first?.type, typeName: parameters.first?.typeName)
                if declared.contains(name) {
                    // A method may be defined again for a different receiver type, not for the same one.
                    let overload = methods.contains(name) && methodReceivers[name]?.contains(receiver) == false
                    if !overload {
                        report("PINE3024", .semantic, "Function '\(name)' is already declared.", range)
                    }
                }
                if methods.contains(name) { methodReceivers[name, default: []].insert(receiver) }
                declared.insert(name)
                // A method may share a builtin's name: it is picked by receiver type.
                if !methods.contains(name) { checkName(name, range) }
                for parameter in parameters { checkName(parameter.name, range) }
                // Function bodies are their own scope: locals may reuse global names, and
                // globals cannot be reassigned from inside a function.
                validate(body, inherited: Set(parameters.map(\.name)), inLoop: false, isNested: true)
            case .conditional, .forRange, .forIn, .whileLoop, .switchStatement:
                validateNested(statement, declared: declared, inLoop: inLoop)
            case .expression(let expression):
                // The last statement of a nested block is its value, so it may be any expression.
                if !(isNested && index == statements.count - 1) { checkStatementExpression(expression) }
            case .typeDeclaration, .enumDeclaration, .fieldAssignment: break
            }
        }
    }

    private mutating func validateNested(
        _ statement: PineStatement, declared: Set<String>, inLoop: Bool
    ) {
        var inherited = declared
        var loop = inLoop
        switch statement {
        case .forRange(let variable, _, _, _, _, let range):
            checkName(variable, range)
            inherited.insert(variable)
            loop = true
        case .forIn(let index, let value, _, _, let range):
            for name in [index, value].compactMap({ $0 }) { checkName(name, range) }
            inherited.formUnion([index, value].compactMap { $0 })
            loop = true
        case .whileLoop: loop = true
        default: break
        }
        for block in statement.nestedBlocks { validate(block, inherited: inherited, inLoop: loop, isNested: true) }
    }

    /// Only calls do something as a statement; a value on its own line is a TradingView syntax error.
    private mutating func checkStatementExpression(_ expression: PineExpression) {
        switch expression {
        case .call, .methodCall, .statementExpression: return
        default: break
        }
        let range = expression.range
        report(
            "PINE3048", .syntax, "\"\(statementText(at: range))\" is not a valid statement.", range)
    }

    /// The statement's source text: from its first token to the end of that line.
    private func statementText(at range: PineSourceRange) -> String {
        let utf16 = source.utf16
        guard let start = utf16.index(utf16.startIndex, offsetBy: range.start.offset, limitedBy: utf16.endIndex),
            let from = start.samePosition(in: source)
        else { return "" }
        let line = source[from...].prefix { !$0.isNewline }
        return line.trimmingCharacters(in: .whitespaces)
    }

    private mutating func declare(_ name: String, _ declared: inout Set<String>, _ range: PineSourceRange) {
        if declared.contains(name) {
            report("PINE3020", .semantic, "Variable '\(name)' is already declared in this scope.", range)
        }
        declared.insert(name)
    }

    private mutating func checkName(_ name: String, _ range: PineSourceRange) {
        guard let violation = PineIdentifierRules.violation(for: name) else { return }
        report(violation.code, .semantic, violation.message, range)
    }

    private mutating func report(
        _ code: String, _ category: PineDiagnosticCategory, _ message: String,
        _ range: PineSourceRange
    ) {
        diagnostics.append(.error(code, category, message, range))
    }
}
