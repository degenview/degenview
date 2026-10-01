import Foundation

extension PineTypeChecker {
    // MARK: - Expressions

    mutating func infer(_ expression: PineExpression, _ scope: Scope) -> Inferred {
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
            return scope[name] ?? PineBuiltinTypes.identifier(name)
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

    static func isNumeric(_ type: PineValueType) -> Bool { type == .int || type == .float }

    static func maxQualifier(_ a: PineQualifier?, _ b: PineQualifier?) -> PineQualifier? {
        guard let a, let b else { return nil }
        return max(a, b)
    }

    /// The common type of two ternary branches, or unknown when they do not agree.
    static func unify(_ a: Inferred, _ b: Inferred, with test: PineQualifier?) -> Inferred {
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
    static func fits(_ value: Inferred, _ declared: PineValueType) -> Bool {
        guard case .known(let type, _) = value else { return true }
        return type == declared || (declared == .float && type == .int)
    }

    static func describe(_ value: Inferred) -> String {
        switch value {
        case .known(let type, _): type.rawValue
        case .na: "na"
        case .unknown: "an unknown type"
        }
    }

    static func describe(_ qualifier: PineQualifier) -> String {
        switch qualifier {
        case .constant: "const"
        case .input: "input"
        case .simple: "simple"
        case .series: "series"
        }
    }

    // MARK: - Operators

    mutating func requireBool(_ value: Inferred, _ what: String, _ range: PineSourceRange) {
        if case .known(let type, _) = value, type != .bool {
            error("PINE3033", "\(what) must be bool, got \(type.rawValue).", range)
        }
    }

    mutating func unary(_ op: PineUnaryOperator, _ operand: Inferred, _ range: PineSourceRange) -> Inferred {
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

    mutating func binary(
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

    mutating func call(
        _ name: String, _ arguments: [PineArgument], _ scope: Scope, _ range: PineSourceRange
    ) -> Inferred {
        // Always walk the arguments so errors inside them are reported, whatever the callee.
        let values = arguments.map { infer($0.value, scope) }
        if functions.contains(name) { return .unknown }
        if let type = PineBuiltinTypes.inputResult(name) {
            if name != "input.source" { checkInputDefault(name, arguments, values) }
            return .known(type, name == "input.source" ? .series : .input)
        }
        if name == "alert" || name == "alertcondition" { checkAlertMessage(name, arguments, values, range) }
        let qualifier = values.reduce(PineQualifier?.some(.constant)) { Self.maxQualifier($0, $1.qualifier) }
        return PineBuiltinTypes.call(name, arguments: arguments, values: values, qualifier: qualifier)
    }

    /// A message whose type is certain and not `string`. Anything the checker cannot type passes.
    mutating func checkAlertMessage(
        _ name: String, _ arguments: [PineArgument], _ values: [Inferred], _ range: PineSourceRange
    ) {
        let position = name == "alert" ? 0 : 2
        let positional = arguments.indices.filter { arguments[$0].name == nil }
        let index =
            arguments.firstIndex { $0.name == "message" } ?? (position < positional.count ? positional[position] : nil)
        guard let index, case .known(let type, _) = values[index], type != .string else { return }
        error("PINE3036", "\(name)() message must be a string, not \(type.rawValue).", range)
    }

    mutating func checkInputDefault(
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
}
