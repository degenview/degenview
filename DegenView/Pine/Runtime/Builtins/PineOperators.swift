import Foundation

/// Pure operator semantics shared by the interpreter and compile-time constant folding.
/// `and`/`or` short-circuit, so the interpreter handles them before it gets here.
enum PineOperators {
    static func apply(
        _ op: PineBinaryOperator, _ a: PineRuntimeValue, _ b: PineRuntimeValue
    ) -> PineRuntimeValue {
        switch op {
        case .add:
            if case .string(let x) = a, case .string(let y) = b { return .string(x + y) }
            return arithmetic(op, a, b)
        case .subtract, .multiply, .divide, .modulo, .power: return arithmetic(op, a, b)
        case .equal, .notEqual, .less, .lessEqual, .greater, .greaterEqual: return compare(op, a, b)
        case .and, .or: return .na
        }
    }

    static func apply(
        _ op: PineAssignmentOperator, old: PineRuntimeValue, _ rhs: PineRuntimeValue
    ) -> PineRuntimeValue {
        switch op {
        case .reassign: rhs
        case .compound(let binary): apply(binary, old, rhs)
        }
    }

    static func negate(_ value: PineRuntimeValue) -> PineRuntimeValue {
        if case .int(let x) = value { return .int(0 &- x) }
        return value.number.map { .float(-$0) } ?? .na
    }

    private static func arithmetic(
        _ op: PineBinaryOperator, _ a: PineRuntimeValue, _ b: PineRuntimeValue
    ) -> PineRuntimeValue {
        if case .int(let x) = a, case .int(let y) = b {
            switch op {
            case .add: return .int(x &+ y)
            case .subtract: return .int(x &- y)
            case .multiply: return .int(x &* y)
            case .modulo: return y == 0 || (x == .min && y == -1) ? .na : .int(x % y)
            default: break
            }
        }
        guard let x = a.number, let y = b.number else { return .na }
        switch op {
        case .add: return .float(x + y)
        case .subtract: return .float(x - y)
        case .multiply: return .float(x * y)
        case .divide: return y == 0 ? .na : .float(x / y)
        case .modulo: return y == 0 ? .na : .float(x.truncatingRemainder(dividingBy: y))
        case .power: return .float(pow(x, y))
        default: return .na
        }
    }

    private static func compare(
        _ op: PineBinaryOperator, _ a: PineRuntimeValue, _ b: PineRuntimeValue
    ) -> PineRuntimeValue {
        if a == .na || b == .na { return .bool(false) }
        if let x = a.number, let y = b.number {
            switch op {
            case .equal: return .bool(x == y)
            case .notEqual: return .bool(x != y)
            case .less: return .bool(x < y)
            case .lessEqual: return .bool(x <= y)
            case .greater: return .bool(x > y)
            case .greaterEqual: return .bool(x >= y)
            default: break
            }
        }
        return .bool(op == .equal ? a == b : a != b)
    }
}
