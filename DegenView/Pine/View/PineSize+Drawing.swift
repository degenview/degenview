import Foundation

extension PineSize {
    /// Radius of a `shape.circle` marker.
    var markerRadius: CGFloat {
        switch self {
        case .tiny: 3
        case .small: 5
        case .normal: 7
        case .large: 10
        case .huge: 14
        case .auto: 5
        }
    }

    /// Point size of label and table text.
    var fontSize: CGFloat {
        switch self {
        case .tiny: 8
        case .small: 10
        case .large: 15
        case .huge: 20
        case .normal, .auto: 12
        }
    }
}
