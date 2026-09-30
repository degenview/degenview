import Foundation

enum PineBarPhase: Sendable {
    case historical
    case realtimeTick(isNew: Bool)
    case realtimeClose(isNew: Bool)
}
