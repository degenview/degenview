import Foundation

/// Static type checker, run after parsing and before the script ever executes.
///
/// It infers each expression as `known(type, qualifier)`, `na`, or `unknown`, and reports an
/// error only when every type involved is known. Anything it cannot infer — unlisted
/// functions, arrays, user-function results, tuples, undeclared names — is `unknown` and
/// passes, so a valid script is never rejected for what the checker does not understand. The
/// runtime's own checks stay in place as a backstop.
///
/// | Code | Rule |
/// |---|---|
/// | `PINE3030` | initializer does not fit the declared type |
/// | `PINE3031` | initializer's qualifier is higher than the declared `const`/`simple` |
/// | `PINE3032` | `:=` / compound assignment changes the variable's type |
/// | `PINE3033` | a condition, ternary test, or `and`/`or`/`not` operand is not bool |
/// | `PINE3034` | operator applied to unsuitable operand types |
/// | `PINE3035` | `input.*` default does not fit the function |
/// | `PINE3036` | `alert()` / `alertcondition()` message is not a string |
struct PineTypeChecker {
    enum Inferred: Equatable {
        /// A qualifier of nil means the type is certain but the qualifier is not.
        case known(PineValueType, PineQualifier?)
        case na
        case unknown

        var type: PineValueType? {
            if case .known(let type, _) = self { return type }
            return nil
        }

        /// `na` is a constant; an unknown value has no certain qualifier.
        var qualifier: PineQualifier? {
            switch self {
            case .known(_, let qualifier): qualifier
            case .na: .constant
            case .unknown: nil
            }
        }
    }

    /// Variable types visible at a point in the script.
    typealias Scope = [String: Inferred]

    private var diagnostics: [PineDiagnostic] = []
    /// Variables that are the target of `:=` somewhere: they can change every bar, so series.
    private var reassigned: Set<String> = []
    var functions: Set<String> = []

    static func check(_ statements: [PineStatement]) -> [PineDiagnostic] {
        var checker = PineTypeChecker()
        checker.reassigned = PineStatement.assignedNames(in: statements)
        checker.functions = Set(
            statements.compactMap { statement -> String? in
                if case .function(let name, _, _, _) = statement { return name }
                return nil
            })
        var scope = Scope()
        checker.check(statements, &scope)
        return checker.diagnostics
    }

    // MARK: - Statements

    private mutating func check(_ statements: [PineStatement], _ scope: inout Scope) {
        for statement in statements {
            switch statement {
            case .declaration(let name, let annotation, let mode, let expression, _):
                declare(name, annotation, mode, expression, &scope)
            case .assignment(let name, let op, let expression, _):
                assign(name, op, expression, scope)
            case .expression(let expression):
                _ = infer(expression, scope)
            case .conditional(let condition, let yes, let no, _):
                requireBool(infer(condition, scope), "if condition", condition.range)
                block(yes, scope)
                block(no, scope)
            case .forRange(let name, let from, let end, let step, let body, _):
                let operands = ([from, end] + (step.map { [$0] } ?? [])).map { infer($0, scope) }
                var inner = scope
                let allInt = operands.allSatisfy { $0.type == .int }
                let anyFloat = operands.contains { $0.type == .float }
                inner[name] = allInt ? .known(.int, .series) : anyFloat ? .known(.float, .series) : .unknown
                check(body, &inner)
            case .forIn(let index, let value, let collection, let body, _):
                _ = infer(collection, scope)
                var inner = scope
                if let index { inner[index] = .known(.int, .series) }
                inner[value] = .unknown
                check(body, &inner)
            case .whileLoop(let condition, let body, _):
                requireBool(infer(condition, scope), "while condition", condition.range)
                block(body, scope)
            case .switchStatement(let subject, let arms, _):
                _ = subject.map { infer($0, scope) }
                for arm in arms {
                    if let condition = arm.condition {
                        let value = infer(condition, scope)
                        if subject == nil { requireBool(value, "switch arm condition", condition.range) }
                    }
                    block(arm.body, scope)
                }
            case .tupleDeclaration(let names, let expression, _):
                _ = infer(expression, scope)
                for name in names { scope[name] = .unknown }
            case .function(let name, let parameters, let body, _):
                var inner = scope
                for parameter in parameters {
                    _ = parameter.defaultValue.map { infer($0, scope) }
                    inner[parameter.name] =
                        parameter.type.flatMap { $0 == .object || $0 == .map ? nil : .known($0, nil) } ?? .unknown
                }
                check(body, &inner)
                scope[name] = .unknown
            case .fieldAssignment(let target, _, let value, _):
                _ = infer(target, scope)
                _ = infer(value, scope)
            case .loopControl, .typeDeclaration, .enumDeclaration: break
            }
        }
    }

    /// Declarations inside a block do not outlive it.
    mutating func block(_ statements: [PineStatement], _ scope: Scope) {
        var inner = scope
        check(statements, &inner)
    }

    private mutating func declare(
        _ name: String, _ annotation: PineTypeAnnotation, _ mode: PineDeclarationMode,
        _ expression: PineExpression, _ scope: inout Scope
    ) {
        let value = infer(expression, scope)
        let series = mode != .ordinary || reassigned.contains(name)

        if let wanted = annotation.qualifier, let actual = value.qualifier, actual > wanted {
            error(
                "PINE3031",
                "Cannot assign a \(Self.describe(actual)) value to a \(Self.describe(wanted)) variable '\(name)'.",
                expression.range)
        }
        if annotation.type == .object || annotation.type == .map {
            // Which type an object is, and whether a value fits it, is not tracked.
            scope[name] = .unknown
        } else if let declared = annotation.type {
            if !Self.fits(value, declared) {
                error(
                    "PINE3030", "Cannot assign \(Self.describe(value)) to \(declared.rawValue) variable '\(name)'.",
                    expression.range)
            }
            scope[name] = .known(
                declared, annotation.qualifier ?? (series ? .series : value.qualifier))
        } else if case .known(let type, let qualifier) = value {
            scope[name] = .known(type, series ? .series : (annotation.qualifier ?? qualifier))
        } else {
            scope[name] = .unknown
        }
    }

    private mutating func assign(
        _ name: String, _ op: PineAssignmentOperator, _ expression: PineExpression, _ scope: Scope
    ) {
        let value = infer(expression, scope)
        guard let target = scope[name], case .known(let type, _) = target else { return }
        var result = value
        if case .compound(let underlying) = op {
            result = binary(
                underlying, target, value, left: expression.range, right: expression.range,
                range: expression.range)
        }
        if !Self.fits(result, type) {
            error(
                "PINE3032", "Cannot assign \(Self.describe(result)) to \(type.rawValue) variable '\(name)'.",
                expression.range)
        }
    }

    mutating func error(_ code: String, _ message: String, _ range: PineSourceRange) {
        diagnostics.append(PineDiagnostic.error(code, .semantic, message, range))
    }
}
