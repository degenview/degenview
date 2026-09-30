import Foundation

enum PineUnaryOperator: Equatable, Sendable {
    case negate, plus, not

    init?(token: PineTokenKind) {
        switch token {
        case .minus: self = .negate
        case .plus: self = .plus
        case .not: self = .not
        default: return nil
        }
    }
}
