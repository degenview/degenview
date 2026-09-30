import Foundation

enum PineLineExtend: String, PineNamedConstant {
    case none, left, right, both
    static let pinePrefix = "extend."

    var extendsLeft: Bool { self == .left || self == .both }
    var extendsRight: Bool { self == .right || self == .both }
}
