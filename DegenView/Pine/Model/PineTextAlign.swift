import Foundation

/// `text.align_*`: how a label's lines, or a box's text, are placed.
enum PineTextAlign: String, PineNamedConstant {
    case left, center, right, top, bottom
    static let pinePrefix = "text.align_"
}
