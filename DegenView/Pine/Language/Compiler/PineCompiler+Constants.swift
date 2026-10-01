import Foundation

extension PineCompiler {
    /// Values of top-level declarations that fold to a constant and are never reassigned, so
    /// `group = g_sr` and `default_qty_value = riskPercent` resolve at compile time.
    static func constantEnvironment(_ statements: [PineStatement]) -> [String: PineRuntimeValue] {
        let reassigned = PineStatement.assignedNames(in: statements)
        var environment: [String: PineRuntimeValue] = [:]
        for statement in statements {
            guard case .declaration(let name, _, .ordinary, let expression, _) = statement,
                !reassigned.contains(name), let value = constantValue(expression, environment)
            else { continue }
            environment[name] = value
        }
        return environment
    }

    /// Folds compile-time expressions: literals, builtin constants, earlier constants,
    /// arithmetic on numbers, string concatenation, colors, and `timestamp()`.
    static func constantValue(
        _ e: PineExpression, _ environment: [String: PineRuntimeValue]
    ) -> PineRuntimeValue? {
        switch e {
        case .literal(let value, _): return value == .na ? nil : value
        case .identifier(let name, _):
            if let value = environment[name] { return value }
            if let value = PineBuiltins.constants[name] { return value }
            return PineBuiltins.colors[name].map(PineRuntimeValue.color)
        case .unary(.negate, let inner, _):
            switch constantValue(inner, environment) {
            case .int(let x)?: return .int(0 &- x)
            case .float(let x)?: return .float(-x)
            default: return nil
            }
        case .binary(let l, let op, let r, _):
            guard op.isArithmetic, let a = constantValue(l, environment),
                let b = constantValue(r, environment)
            else { return nil }
            let value = PineOperators.apply(op, a, b)
            return value == .na ? nil : value
        case .call("timestamp", let arguments, _, _):
            return constantTimestamp(arguments, environment)
        case .call("color.new", _, _, _), .call("color.rgb", _, _, _):
            return constantColor(e, environment).map(PineRuntimeValue.color)
        default: return nil
        }
    }

    private static func constantTimestamp(
        _ arguments: [PineArgument], _ environment: [String: PineRuntimeValue]
    ) -> PineRuntimeValue? {
        var positional: [PineRuntimeValue] = []
        var named: [String: PineRuntimeValue] = [:]
        for argument in arguments {
            guard let value = constantValue(argument.value, environment) else { return nil }
            if let name = argument.name { named[name] = value } else { positional.append(value) }
        }
        return PineTimestamp.evaluate(positional: positional, named: named).map(PineRuntimeValue.int)
    }

    /// Folds compile-time color expressions: literals, `color.*` constants, earlier constants, and
    /// `color.new`/`color.rgb` with constant arguments (`color.new(BASE, 88)`).
    static func constantColor(_ e: PineExpression, _ environment: [String: PineRuntimeValue]) -> UInt32? {
        switch e {
        case .literal(.color(let rgba), _): return rgba
        case .identifier:
            if case .color(let rgba)? = constantValue(e, environment) { return rgba }
            return nil
        case .call("color.new", let arguments, _, _):
            guard arguments.count >= 2, let base = constantColor(arguments[0].value, environment),
                let transparency = constantNumber(arguments[1].value, environment)
            else { return nil }
            return PineBuiltins.withTransparency(base, transparency)
        case .call("color.rgb", let arguments, _, _):
            let numbers = arguments.compactMap { constantNumber($0.value, environment) }
            guard numbers.count == arguments.count, numbers.count >= 3 else { return nil }
            return PineBuiltins.rgb(numbers[0], numbers[1], numbers[2], numbers.count > 3 ? numbers[3] : 0)
        default: return nil
        }
    }

    private static func constantNumber(
        _ e: PineExpression, _ environment: [String: PineRuntimeValue]
    ) -> Double? {
        switch e {
        case .literal(let value, _): return value.number
        case .identifier: return constantValue(e, environment)?.number
        case .unary(.negate, let inner, _): return constantNumber(inner, environment).map { -$0 }
        default: return nil
        }
    }
}
