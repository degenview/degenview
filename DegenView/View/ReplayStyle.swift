import SwiftUI

/// What Historical Replay draws with, in one place: the accent that marks a time-travelled chart,
/// the per-state colours, and the date/time formats. System colours only, so both themes stay legible.
enum ReplayStyle {
    static let accent = Color.orange
    static let cornerRadius: CGFloat = 7

    static func tint(for status: ReplayStatus, isPreparing: Bool) -> Color {
        if isPreparing { return .secondary }
        switch status {
        case .playing: return .green
        case .completed: return .secondary
        case .inactive, .selectingStart, .paused: return accent
        }
    }

    static func title(for status: ReplayStatus, isPreparing: Bool) -> String {
        if isPreparing { return "Loading history" }
        switch status {
        case .inactive: return "Off"
        case .selectingStart: return "Pick a start"
        case .paused: return "Paused"
        case .playing: return "Playing"
        case .completed: return "Ended"
        }
    }

    static func dateText(_ date: Date) -> String {
        date.formatted(.dateTime.year().month(.abbreviated).day())
    }

    static func timeText(_ date: Date) -> String {
        date.formatted(.dateTime.hour().minute().second())
    }

    static func compactText(_ date: Date) -> String {
        date.formatted(.dateTime.month(.abbreviated).day().hour().minute())
    }

    static func fullText(_ date: Date) -> String {
        date.formatted(date: .abbreviated, time: .standard)
    }
}
