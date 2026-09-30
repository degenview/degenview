import Foundation

enum PineBarPhase: Sendable {
    case historical
    case realtimeTick(isNew: Bool)
    case realtimeClose(isNew: Bool)

    /// Whether the bar's values are final.
    var isConfirmed: Bool {
        if case .realtimeTick = self { return false }
        return true
    }

    var isRealtime: Bool {
        if case .historical = self { return false }
        return true
    }
}
