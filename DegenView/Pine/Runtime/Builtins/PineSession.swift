import Foundation

/// A Pine session string such as `"0930-1600"`, `"1800-0600"` (overnight), `"0930-1130,1300-1600"`
/// or `"0930-1600:23456"` (Monday to Friday; Sunday is 1). `"24x7"` is always open.
struct PineSession: Equatable {
    private struct Window: Equatable {
        var start: Int
        var end: Int
        var days: Set<Int>

        func contains(minute: Int, weekday: Int) -> Bool {
            if start == end { return days.contains(weekday) }
            if start < end { return minute >= start && minute < end && days.contains(weekday) }
            if minute >= start { return days.contains(weekday) }
            // The after-midnight part of an overnight window belongs to the day it opened on.
            return minute < end && days.contains(weekday == 1 ? 7 : weekday - 1)
        }
    }

    private static let minutesPerDay = 1440
    private static let allDays = Set(1...7)
    private var windows: [Window]

    /// Nil when the text is not a session.
    init?(_ text: String) {
        let compact = text.filter { !$0.isWhitespace }
        if compact.lowercased() == "24x7" {
            windows = [.init(start: 0, end: 0, days: Self.allDays)]
            return
        }
        let parsed = compact.split(separator: ",", omittingEmptySubsequences: false).map(Self.window)
        guard !parsed.isEmpty, !parsed.contains(where: { $0 == nil }) else { return nil }
        windows = parsed.compactMap { $0 }
    }

    func contains(_ components: PineCalendar.Components) -> Bool {
        let minute = components.hour * 60 + components.minute
        return windows.contains { $0.contains(minute: minute, weekday: components.weekday) }
    }

    private static func window(_ text: Substring) -> Window? {
        let sides = text.split(separator: ":", omittingEmptySubsequences: false)
        guard sides.count <= 2 else { return nil }
        let range = sides[0].split(separator: "-", omittingEmptySubsequences: false)
        guard range.count == 2, let start = minuteOfDay(range[0]), let end = minuteOfDay(range[1]) else {
            return nil
        }
        var days = allDays
        if sides.count == 2 {
            let digits = sides[1].compactMap { $0.wholeNumberValue }
            guard !digits.isEmpty, digits.count == sides[1].count, digits.allSatisfy({ (1...7).contains($0) })
            else { return nil }
            days = Set(digits)
        }
        return Window(start: start, end: end, days: days)
    }

    private static func minuteOfDay(_ text: Substring) -> Int? {
        guard text.count == 4, let value = Int(text) else { return nil }
        let (hour, minute) = (value / 100, value % 100)
        guard minute < 60, hour <= 24, hour < 24 || minute == 0 else { return nil }
        return hour * 60 + minute
    }
}
