import Foundation

enum PineLineStyle: String, PineNamedConstant {
    case solid, dotted, dashed
    case arrowLeft = "arrow_left"
    case arrowRight = "arrow_right"
    case arrowBoth = "arrow_both"
    static let pinePrefix = "line.style_"
}
