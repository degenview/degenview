import SwiftUI

/// Where the Script Manager puts the preview chart relative to the code editor.
enum ChartPosition: String, CaseIterable, Identifiable {
    case left, top, bottom

    var id: String { rawValue }

    var title: String {
        switch self {
        case .left: "Left of Code"
        case .top: "Above Code"
        case .bottom: "Below Code"
        }
    }

    var symbol: String {
        switch self {
        case .left: "rectangle.lefthalf.inset.filled"
        case .top: "rectangle.tophalf.inset.filled"
        case .bottom: "rectangle.bottomhalf.inset.filled"
        }
    }

    /// The axis along which the chart and the editor are split.
    var axis: Axis {
        self == .left ? .horizontal : .vertical
    }

    /// Whether the chart comes before the editor — left of it, or above it.
    var chartFirst: Bool { self != .bottom }
}
