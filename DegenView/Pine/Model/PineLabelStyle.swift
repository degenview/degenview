import Foundation

/// `label.style_*`. `PineLabelGeometry` says how each is drawn; `textOutline` is plain text (the outline is
/// not drawn) and the glyph styles are simplified markers.
enum PineLabelStyle: String, PineNamedConstant {
    case none
    case labelDown = "label_down"
    case labelUp = "label_up"
    case labelLeft = "label_left"
    case labelRight = "label_right"
    case labelCenter = "label_center"
    case labelLowerLeft = "label_lower_left"
    case labelLowerRight = "label_lower_right"
    case labelUpperLeft = "label_upper_left"
    case labelUpperRight = "label_upper_right"
    case circle, square, diamond, cross, xcross, flag, triangleup, triangledown, arrowup, arrowdown
    case textOutline = "text_outline"
    static let pinePrefix = "label.style_"
}
