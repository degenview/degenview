import Foundation

enum ReplayStatus: String, Codable, Equatable {
    case inactive
    case selectingStart
    case paused
    case playing
    case completed
}

enum ReplayInterval: String, CaseIterable, Codable, Identifiable {
    case automatic = "Auto"
    case oneMinute = "1m"
    case fiveMinutes = "5m"
    case fifteenMinutes = "15m"
    case thirtyMinutes = "30m"
    case oneHour = "1h"
    case oneDay = "1D"
    case chartBar = "1 bar"

    var id: String { rawValue }

    var seconds: TimeInterval? {
        switch self {
        case .automatic, .chartBar: return nil
        case .oneMinute: return 60
        case .fiveMinutes: return 300
        case .fifteenMinutes: return 900
        case .thirtyMinutes: return 1_800
        case .oneHour: return 3_600
        case .oneDay: return 86_400
        }
    }

    var apiInterval: String? {
        switch self {
        case .automatic, .chartBar: return nil
        case .oneMinute: return "1m"
        case .fiveMinutes: return "5m"
        case .fifteenMinutes: return "15m"
        case .thirtyMinutes: return "30m"
        case .oneHour: return "1h"
        case .oneDay: return "1d"
        }
    }
}

enum ReplaySpeed: String, CaseIterable, Codable, Identifiable {
    case half = "0.5×"
    case normal = "1×"
    case double = "2×"
    case five = "5×"
    case ten = "10×"
    case twenty = "20×"
    case fifty = "50×"
    case maximum = "Max"

    var id: String { rawValue }

    var delayNanoseconds: UInt64 {
        switch self {
        case .half: return 2_000_000_000
        case .normal: return 1_000_000_000
        case .double: return 500_000_000
        case .five: return 200_000_000
        case .ten: return 100_000_000
        case .twenty: return 50_000_000
        case .fifty: return 20_000_000
        case .maximum: return 1_000_000
        }
    }
}

struct ReplaySession: Codable, Equatable {
    var status: ReplayStatus
    var symbol: String
    var chartTimeframe: TimeRange
    var startTimestamp: Date
    var currentTimestamp: Date
    var currentBarIndex: Int
    var replayInterval: ReplayInterval
    var playbackSpeed: ReplaySpeed
    var sessionStartedAt: Date

    var restoredPaused: ReplaySession {
        var copy = self
        copy.status = copy.status == .completed ? .completed : .paused
        return copy
    }
}
