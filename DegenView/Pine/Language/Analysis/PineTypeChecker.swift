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

    private struct Scope {
        var variables: [String: Inferred] = [:]
    }

    private var diagnostics: [PineDiagnostic] = []
    /// Variables that are the target of `:=` somewhere: they can change every bar, so series.
    private var reassigned: Set<String> = []
    private var functions: Set<String> = []

    static func check(_ statements: [PineStatement]) -> [PineDiagnostic] {
        var checker = PineTypeChecker()
        checker.reassigned = reassignedNames(in: statements)
        checker.functions = Set(
            statements.compactMap { statement -> String? in
                if case .function(let name, _, _, _) = statement { return name }
                return nil
            })
        var scope = Scope()
        checker.check(statements, &scope)
        return checker.diagnostics
    }

    private static func reassignedNames(in statements: [PineStatement]) -> Set<String> {
        var names = Set<String>()
        for statement in statements {
            switch statement {
            case .assignment(let name, _, _, _): names.insert(name)
            case .conditional(_, let yes, let no, _):
                names.formUnion(reassignedNames(in: yes))
                names.formUnion(reassignedNames(in: no))
            case .forRange(_, _, _, _, let body, _), .forIn(_, _, _, let body, _), .whileLoop(_, let body, _),
                .function(_, _, let body, _):
                names.formUnion(reassignedNames(in: body))
            case .switchStatement(_, let arms, _):
                for arm in arms { names.formUnion(reassignedNames(in: arm.body)) }
            default: break
            }
        }
        return names
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
                let bounds = [from, end].map { infer($0, scope) }
                let stride = step.map { infer($0, scope) }
                var inner = scope
                let allInt = (bounds + (stride.map { [$0] } ?? [])).allSatisfy { $0.type == .int }
                let anyFloat = (bounds + (stride.map { [$0] } ?? [])).contains { $0.type == .float }
                inner.variables[name] = allInt ? .known(.int, .series) : anyFloat ? .known(.float, .series) : .unknown
                check(body, &inner)
            case .forIn(let index, let value, let collection, let body, _):
                _ = infer(collection, scope)
                var inner = scope
                if let index { inner.variables[index] = .known(.int, .series) }
                inner.variables[value] = .unknown
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
                for name in names { scope.variables[name] = .unknown }
            case .function(let name, let parameters, let body, _):
                var inner = scope
                for parameter in parameters {
                    _ = parameter.defaultValue.map { infer($0, scope) }
                    inner.variables[parameter.name] = parameter.type.map { .known($0, nil) } ?? .unknown
                }
                check(body, &inner)
                scope.variables[name] = .unknown
            case .loopControl: break
            }
        }
    }

    /// Declarations inside a block do not outlive it.
    private mutating func block(_ statements: [PineStatement], _ scope: Scope) {
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
        if let declared = annotation.type {
            if !Self.fits(value, declared) {
                error(
                    "PINE3030", "Cannot assign \(Self.describe(value)) to \(declared.rawValue) variable '\(name)'.",
                    expression.range)
            }
            scope.variables[name] = .known(
                declared, annotation.qualifier ?? (series ? .series : value.qualifier))
        } else if case .known(let type, let qualifier) = value {
            scope.variables[name] = .known(type, series ? .series : (annotation.qualifier ?? qualifier))
        } else {
            scope.variables[name] = .unknown
        }
    }

    private mutating func assign(
        _ name: String, _ op: PineAssignmentOperator, _ expression: PineExpression, _ scope: Scope
    ) {
        let value = infer(expression, scope)
        guard let target = scope.variables[name], case .known(let type, _) = target else { return }
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

    // MARK: - Expressions

    private mutating func infer(_ expression: PineExpression, _ scope: Scope) -> Inferred {
        switch expression {
        case .literal(let value, _):
            switch value {
            case .int: return .known(.int, .constant)
            case .float: return .known(.float, .constant)
            case .bool: return .known(.bool, .constant)
            case .string: return .known(.string, .constant)
            case .color: return .known(.color, .constant)
            case .na: return .na
            default: return .unknown
            }
        case .identifier(let name, _):
            return Self.identifier(name, scope)
        case .unary(let op, let operand, let range):
            return unary(op, infer(operand, scope), range)
        case .binary(let left, let op, let right, let range):
            let a = infer(left, scope)
            let b = infer(right, scope)
            return binary(op, a, b, left: left.range, right: right.range, range: range)
        case .ternary(let condition, let yes, let no, _):
            let test = infer(condition, scope)
            let a = infer(yes, scope)
            let b = infer(no, scope)
            requireBool(test, "Ternary condition", condition.range)
            guard test != .unknown else { return .unknown }
            return Self.unify(a, b, with: test.qualifier)
        case .call(let name, let arguments, _, let range):
            return call(name, arguments, scope, range)
        case .history(let base, let offset, _):
            let value = infer(base, scope)
            _ = infer(offset, scope)
            if case .known(let type, _) = value { return .known(type, .series) }
            return .unknown
        case .tuple(let items, _):
            for item in items { _ = infer(item, scope) }
            return .unknown
        case .statementExpression(let statement, _):
            block([statement], scope)
            return .unknown
        }
    }

    private static func identifier(_ name: String, _ scope: Scope) -> Inferred {
        if let variable = scope.variables[name] { return variable }
        switch PineBuiltins.constants[name] {
        case .int?: return .known(.int, .constant)
        case .float?: return .known(.float, .constant)
        case .string?: return .known(.string, .constant)
        default: break
        }
        if PineBuiltins.colors[name] != nil { return .known(.color, .constant) }
        switch name {
        case "open", "high", "low", "close", "volume", "hl2", "hlc3", "ohlc4", "ta.tr",
            "strategy.position_size", "strategy.position_avg_price", "strategy.equity", "strategy.netprofit",
            "strategy.openprofit", "strategy.initial_capital", "strategy.grossprofit", "strategy.grossloss":
            return .known(.float, .series)
        case "time", "time_close", "bar_index", "year", "month", "dayofmonth", "hour", "minute", "second",
            "dayofweek", "strategy.closedtrades", "strategy.opentrades", "strategy.wintrades",
            "strategy.losstrades":
            return .known(.int, .series)
        case "syminfo.mintick": return .known(.float, .simple)
        case "syminfo.ticker", "syminfo.tickerid", "syminfo.currency", "syminfo.type":
            return .known(.string, .simple)
        case "chart.fg_color", "chart.bg_color": return .known(.color, .simple)
        default: return name.hasPrefix("barstate.") ? .known(.bool, .series) : .unknown
        }
    }

    private static func isNumeric(_ type: PineValueType) -> Bool { type == .int || type == .float }

    private static func maxQualifier(_ a: PineQualifier?, _ b: PineQualifier?) -> PineQualifier? {
        guard let a, let b else { return nil }
        return max(a, b)
    }

    /// The common type of two ternary branches, or unknown when they do not agree.
    private static func unify(_ a: Inferred, _ b: Inferred, with test: PineQualifier?) -> Inferred {
        switch (a, b) {
        case (.known(let x, let qx), .known(let y, let qy)):
            let qualifier = maxQualifier(test, maxQualifier(qx, qy))
            if x == y { return .known(x, qualifier) }
            if isNumeric(x) && isNumeric(y) { return .known(.float, qualifier) }
            return .unknown
        case (.known(let x, let qx), .na), (.na, .known(let x, let qx)):
            return .known(x, maxQualifier(test, qx))
        case (.na, .na): return .na
        default: return .unknown
        }
    }

    /// Whether `value` may be stored in a variable of type `declared`. `na` always fits: the
    /// v6 rule that a bool cannot be `na` is `PINE3021`'s job.
    private static func fits(_ value: Inferred, _ declared: PineValueType) -> Bool {
        guard case .known(let type, _) = value else { return true }
        return type == declared || (declared == .float && type == .int)
    }

    private static func describe(_ value: Inferred) -> String {
        switch value {
        case .known(let type, _): type.rawValue
        case .na: "na"
        case .unknown: "an unknown type"
        }
    }

    private static func describe(_ qualifier: PineQualifier) -> String {
        switch qualifier {
        case .constant: "const"
        case .input: "input"
        case .simple: "simple"
        case .series: "series"
        }
    }

    // MARK: - Operators

    private mutating func requireBool(_ value: Inferred, _ what: String, _ range: PineSourceRange) {
        if case .known(let type, _) = value, type != .bool {
            error("PINE3033", "\(what) must be bool, got \(type.rawValue).", range)
        }
    }

    private mutating func unary(_ op: PineUnaryOperator, _ operand: Inferred, _ range: PineSourceRange) -> Inferred {
        guard case .known(let type, let qualifier) = operand else { return .unknown }
        if op == .not {
            requireBool(operand, "Operand of 'not'", range)
            return .known(.bool, qualifier)
        }
        guard Self.isNumeric(type) else {
            error("PINE3034", "Operator '\(op == .negate ? "-" : "+")' cannot be applied to \(type.rawValue).", range)
            return .unknown
        }
        return operand
    }

    private mutating func binary(
        _ op: PineBinaryOperator, _ a: Inferred, _ b: Inferred, left: PineSourceRange, right: PineSourceRange,
        range: PineSourceRange
    ) -> Inferred {
        // A comparison with `na` is legal whatever the other side is.
        if op.isComparison, a == .na || b == .na { return .known(.bool, .series) }
        guard case .known(let x, let qx) = a, case .known(let y, let qy) = b else { return .unknown }
        let qualifier = Self.maxQualifier(qx, qy)

        switch op {
        case .and, .or:
            requireBool(a, "Operand of '\(op.symbol)'", left)
            requireBool(b, "Operand of '\(op.symbol)'", right)
            return .known(.bool, qualifier)
        case .equal, .notEqual:
            if x == y || (Self.isNumeric(x) && Self.isNumeric(y)) { return .known(.bool, qualifier) }
        case .less, .lessEqual, .greater, .greaterEqual:
            if Self.isNumeric(x) && Self.isNumeric(y) { return .known(.bool, qualifier) }
        case .add:
            if x == .string && y == .string { return .known(.string, qualifier) }
            if Self.isNumeric(x) && Self.isNumeric(y) {
                return .known(x == .int && y == .int ? .int : .float, qualifier)
            }
        case .subtract, .multiply, .modulo:
            if Self.isNumeric(x) && Self.isNumeric(y) {
                return .known(x == .int && y == .int ? .int : .float, qualifier)
            }
        case .divide, .power:
            if Self.isNumeric(x) && Self.isNumeric(y) { return .known(.float, qualifier) }
        }
        error(
            "PINE3034", "Operator '\(op.symbol)' cannot be applied to \(x.rawValue) and \(y.rawValue).", range)
        return .unknown
    }

    // MARK: - Calls

    private mutating func call(
        _ name: String, _ arguments: [PineArgument], _ scope: Scope, _ range: PineSourceRange
    ) -> Inferred {
        // Always walk the arguments so errors inside them are reported, whatever the callee.
        let values = arguments.map { infer($0.value, scope) }
        if functions.contains(name) { return .unknown }
        let qualifier = values.reduce(PineQualifier?.some(.constant)) { Self.maxQualifier($0, $1.qualifier) }

        switch name {
        case "input.int", "input.float", "input.bool", "input.string", "input.color", "input.time":
            checkInputDefault(name, arguments, values)
            let type: PineValueType =
                switch name {
                case "input.float": .float
                case "input.bool": .bool
                case "input.string": .string
                case "input.color": .color
                default: .int
                }
            return .known(type, .input)
        case "input.source": return .known(.float, .series)
        case "int": return .known(.int, qualifier)
        case "float": return .known(.float, qualifier)
        case "bool", "na": return .known(.bool, qualifier)
        case "timestamp": return .known(.int, qualifier)
        case "color.new", "color.rgb": return .known(.color, qualifier)
        case "nz":
            if let type = values.first?.type, Self.isNumeric(type) { return .known(type, qualifier) }
            return .unknown
        case "str.tostring", "str.format", "str.upper", "str.lower", "str.trim", "str.replace_all",
            "str.substring":
            return .known(.string, qualifier)
        case "str.length": return .known(.int, qualifier)
        case "str.contains", "str.startswith", "str.endswith": return .known(.bool, qualifier)
        case "str.tonumber": return .known(.float, qualifier)
        case "math.round":
            return .known(arguments.count > 1 ? .float : .int, qualifier)
        case "math.floor", "math.ceil": return .known(.int, qualifier)
        case "math.max", "math.min":
            let types = values.compactMap(\.type)
            guard types.count == values.count, !types.isEmpty, types.allSatisfy(Self.isNumeric) else {
                return .unknown
            }
            return .known(types.allSatisfy { $0 == .int } ? .int : .float, qualifier)
        case "math.abs":
            if let type = values.first?.type, Self.isNumeric(type) { return .known(type, qualifier) }
            return .unknown
        case "math.sqrt", "math.pow", "math.log", "math.log10", "math.exp", "math.sin", "math.cos",
            "math.tan", "math.asin", "math.acos", "math.atan", "math.avg", "math.todegrees",
            "math.toradians", "math.round_to_mintick":
            return .known(.float, qualifier)
        case "line.new": return .known(.line, .series)
        case "label.new": return .known(.label, .series)
        case "box.new": return .known(.box, .series)
        case "table.new": return .known(.table, .series)
        case "array.from", "array.copy", "array.slice": return .known(.array, .series)
        case "ta.sma", "ta.ema", "ta.rma", "ta.wma", "ta.rsi", "ta.atr", "ta.tr", "ta.stdev", "ta.highest",
            "ta.lowest", "ta.mom", "ta.roc", "ta.cum", "ta.pivothigh", "ta.pivotlow":
            return .known(.float, .series)
        case "ta.cross", "ta.crossover", "ta.crossunder", "ta.rising", "ta.falling":
            return .known(.bool, .series)
        default:
            return name.hasPrefix("array.new_") ? .known(.array, .series) : .unknown
        }
    }

    private mutating func checkInputDefault(
        _ name: String, _ arguments: [PineArgument], _ values: [Inferred]
    ) {
        guard let index = arguments.firstIndex(where: { $0.name == nil || $0.name == "defval" }),
            case .known(let type, _) = values[index]
        else { return }
        let accepted: [PineValueType]
        switch name {
        case "input.float": accepted = [.float, .int]
        case "input.bool": accepted = [.bool]
        case "input.string": accepted = [.string]
        case "input.color": accepted = [.color]
        default: accepted = [.int]
        }
        if !accepted.contains(type) {
            error(
                "PINE3035", "\(name)() default must be \(accepted[0].rawValue), got \(type.rawValue).",
                arguments[index].value.range)
        }
    }

    private mutating func error(_ code: String, _ message: String, _ range: PineSourceRange) {
        diagnostics.append(PineDiagnostic.error(code, .semantic, message, range))
    }
}
