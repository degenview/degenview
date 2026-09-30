import Foundation

enum PineMarkerLocation: String, PineNamedConstant {
    case abovebar, belowbar, top, bottom, absolute
    static let pinePrefix = "location."

    var isBelow: Bool { self == .belowbar || self == .bottom }
}
