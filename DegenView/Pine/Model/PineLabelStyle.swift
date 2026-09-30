import Foundation

/// `label.style_*`. Styles without a dedicated bubble shape are drawn as `.labelCenter`.
enum PineLabelStyle: String, PineNamedConstant {
    case none
    case labelDown = "label_down"
    case labelUp = "label_up"
    case labelLeft = "label_left"
    case labelRight = "label_right"
    case labelCenter = "label_center"
    static let pinePrefix = "label.style_"
}
