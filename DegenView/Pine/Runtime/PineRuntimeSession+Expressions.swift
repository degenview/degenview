import Foundation

extension PineRuntimeSession {
    func eval(
        _ expression: PineExpression, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        try budget()
        switch expression {
        case .literal(let value, _): return value
        case .identifier(let name, let range): return try resolveIdentifier(name, range, context)
        case .unary(let op, let operand, let range): return try evalUnary(op, operand, range, &context)
        case .binary(let left, let op, let right, let range):
            return try evalBinary(left, op, right, range, &context)
        case .ternary(let condition, let whenTrue, let whenFalse, let range):
            guard case .bool(let test) = try eval(condition, &context) else {
                throw PineDiagnostic.error(
                    "PINE4005", .runtime, "Ternary condition must be bool.", range)
            }
            return try eval(test ? whenTrue : whenFalse, &context)
        case .history(let base, let offset, let range): return try evalHistory(base, offset, range, &context)
        case .tuple(let expressions, _): return .tuple(try expressions.map { try eval($0, &context) })
        case .member(let base, let name, let range):
            return try readField(name, of: try eval(base, &context), range)
        case .statementExpression(let statement, _):
            let (_, value) = try run([statement], &context)
            return value == .void ? .na : value
        case .call(let name, let arguments, let site, let range):
            return try call(
                PineCall(name: name, arguments: arguments, site: site, range: range), &context)
        }
    }

    // MARK: - Identifiers

    /// Variables first, then series and symbol facts, `barstate.*`, colors and named constants.
    /// A dotted name that matches nothing is an enumeration constant (`size.small`,
    /// `shape.circle`…) and stands for itself; a plain name that matches nothing is a typo.
    private func resolveIdentifier(
        _ name: String, _ range: PineSourceRange, _ context: PineRuntimeContext
    ) throws -> PineRuntimeValue {
        if let value = working.variables[name] { return value }
        if let value = market(name, context) { return value }
        if let flag = context.flags.value(named: name) { return .bool(flag) }
        if let color = PineBuiltins.colors[name] ?? chartColor(name) { return .color(color) }
        if let constant = PineBuiltins.constants[name] { return constant }
        guard name.contains(".") else {
            throw PineDiagnostic.error("PINE4008", .runtime, "Undefined variable '\(name)'.", range)
        }
        if let field = try fieldPath(name, range) { return field }
        return .string(name)
    }

    private func chartColor(_ name: String) -> UInt32? {
        switch name {
        case "chart.fg_color": theme.foreground
        case "chart.bg_color": theme.background
        default: nil
        }
    }

    // MARK: - Operators

    private func evalUnary(
        _ op: PineUnaryOperator, _ operand: PineExpression, _ range: PineSourceRange,
        _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let value = try eval(operand, &context)
        switch op {
        case .negate: return PineOperators.negate(value)
        case .plus: return value
        case .not:
            guard case .bool(let b) = value else {
                throw PineDiagnostic.error("PINE4002", .runtime, "not requires bool.", range)
            }
            return .bool(!b)
        }
    }

    private func evalBinary(
        _ left: PineExpression, _ op: PineBinaryOperator, _ right: PineExpression,
        _ range: PineSourceRange, _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let lhs = try eval(left, &context)
        switch op {
        case .and: return try evalLogical(isAnd: true, lhs, right, range, &context)
        case .or: return try evalLogical(isAnd: false, lhs, right, range, &context)
        default: return PineOperators.apply(op, lhs, try eval(right, &context))
        }
    }

    /// `and`/`or` evaluate the right side only when the left does not decide the result.
    private func evalLogical(
        isAnd: Bool, _ lhs: PineRuntimeValue, _ right: PineExpression, _ range: PineSourceRange,
        _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        let word = isAnd ? "and" : "or"
        let error = PineDiagnostic.error(
            isAnd ? "PINE4003" : "PINE4004", .runtime, "\(word) requires bool operands.", range)
        guard case .bool(let left) = lhs else { throw error }
        if left != isAnd { return .bool(left) }
        guard case .bool(let rhs) = try eval(right, &context) else { throw error }
        return .bool(rhs)
    }

    // MARK: - History

    /// `series[n]`: the value `n` confirmed bars ago.
    private func evalHistory(
        _ base: PineExpression, _ offset: PineExpression, _ range: PineSourceRange,
        _ context: inout PineRuntimeContext
    ) throws -> PineRuntimeValue {
        guard case .identifier(let name, _) = base, let n = try eval(offset, &context).number,
            n.isFinite
        else {
            throw PineDiagnostic.error(
                "PINE4006", .runtime,
                "History offset must be a non-negative integer and base must be a series.", range)
        }
        let negative = PineDiagnostic.error(
            "PINE4006", .runtime, "History offset cannot be negative.", range)
        // An offset beyond exact-integer range is necessarily past the start of any history.
        guard let i = Int(pine: n) else {
            if n < 0 { throw negative }
            return .na
        }
        guard i >= 0 else { throw negative }
        if i == 0 { return try eval(base, &context) }
        let history = working.histories[name] ?? []
        return i <= history.count ? history[history.count - i] : .na
    }
}
