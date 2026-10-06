import Foundation

extension PineAlertFrequency {
    /// How a call's frequency reads in the UI, rather than its Pine constant.
    var displayName: String {
        switch self {
        case .all: "Every update"
        case .oncePerBar: "Once per bar"
        case .oncePerBarClose: "Once per bar close"
        }
    }
}
