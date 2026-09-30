import Foundation

enum PineTablePosition: String, PineNamedConstant {
    case topLeft = "top_left"
    case topCenter = "top_center"
    case topRight = "top_right"
    case middleLeft = "middle_left"
    case middleCenter = "middle_center"
    case middleRight = "middle_right"
    case bottomLeft = "bottom_left"
    case bottomCenter = "bottom_center"
    case bottomRight = "bottom_right"
    static let pinePrefix = "position."

    enum Vertical { case top, middle, bottom }
    enum Horizontal { case left, center, right }

    var vertical: Vertical {
        switch self {
        case .topLeft, .topCenter, .topRight: .top
        case .middleLeft, .middleCenter, .middleRight: .middle
        case .bottomLeft, .bottomCenter, .bottomRight: .bottom
        }
    }

    var horizontal: Horizontal {
        switch self {
        case .topLeft, .middleLeft, .bottomLeft: .left
        case .topCenter, .middleCenter, .bottomCenter: .center
        case .topRight, .middleRight, .bottomRight: .right
        }
    }
}
