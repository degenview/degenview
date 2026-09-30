import Foundation

enum PineMarkerShape: String, PineNamedConstant {
    case xcross, cross, circle, triangleup, triangledown, flag, arrowup, arrowdown
    case labelup, labeldown, square, diamond
    static let pinePrefix = "shape."

    var pointsDown: Bool { self == .triangledown || self == .arrowdown || self == .labeldown }
}
