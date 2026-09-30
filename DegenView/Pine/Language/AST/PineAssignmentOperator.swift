import Foundation

enum PineAssignmentOperator: Equatable, Sendable {
    /// `:=`
    case reassign
    /// `+=`, `-=`, `*=`, `/=`
    case compound(PineBinaryOperator)

    init?(token: PineTokenKind) {
        switch token {
        case .reassign: self = .reassign
        case .plusAssign: self = .compound(.add)
        case .minusAssign: self = .compound(.subtract)
        case .starAssign: self = .compound(.multiply)
        case .slashAssign: self = .compound(.divide)
        default: return nil
        }
    }
}
