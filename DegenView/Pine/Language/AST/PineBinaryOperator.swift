import Foundation

enum PineBinaryOperator: Equatable, Sendable {
    case or, and
    case equal, notEqual, less, lessEqual, greater, greaterEqual
    case add, subtract, multiply, divide, modulo, power

    init?(token: PineTokenKind) {
        switch token {
        case .or: self = .or
        case .and: self = .and
        case .equal: self = .equal
        case .notEqual: self = .notEqual
        case .less: self = .less
        case .lessEqual: self = .lessEqual
        case .greater: self = .greater
        case .greaterEqual: self = .greaterEqual
        case .plus: self = .add
        case .minus: self = .subtract
        case .star: self = .multiply
        case .slash: self = .divide
        case .percent: self = .modulo
        case .power: self = .power
        default: return nil
        }
    }

    var symbol: String {
        switch self {
        case .or: "or"
        case .and: "and"
        case .equal: "=="
        case .notEqual: "!="
        case .less: "<"
        case .lessEqual: "<="
        case .greater: ">"
        case .greaterEqual: ">="
        case .add: "+"
        case .subtract: "-"
        case .multiply: "*"
        case .divide: "/"
        case .modulo: "%"
        case .power: "**"
        }
    }

    var isComparison: Bool {
        switch self {
        case .equal, .notEqual, .less, .lessEqual, .greater, .greaterEqual: true
        default: false
        }
    }

    var isArithmetic: Bool {
        switch self {
        case .add, .subtract, .multiply, .divide, .modulo, .power: true
        default: false
        }
    }

    /// Pratt-parser binding powers. `power` is right-associative.
    var bindingPower: (left: Int, right: Int) {
        switch self {
        case .or: (10, 11)
        case .and: (20, 21)
        case .equal, .notEqual: (30, 31)
        case .less, .lessEqual, .greater, .greaterEqual: (40, 41)
        case .add, .subtract: (50, 51)
        case .multiply, .divide, .modulo: (60, 61)
        case .power: (70, 70)
        }
    }
}
