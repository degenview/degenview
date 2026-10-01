import Foundation

/// Calendar parts of a bar time, and `timeframe.*` derived from the spacing between bars.
enum PineTime {
    static let partNames: Set<String> = [
        "year", "month", "dayofmonth", "hour", "minute", "second", "dayofweek",
    ]

    /// Every `timeframe.*` name `timeframe(_:barSeconds:)` resolves.
    static let timeframeNames: Set<String> = [
        "timeframe.period", "timeframe.multiplier", "timeframe.isdaily", "timeframe.isweekly",
        "timeframe.ismonthly", "timeframe.isseconds", "timeframe.isminutes",
        "timeframe.isintraday", "timeframe.isdwm",
    ]

    private static let day = 86_400.0

    static func milliseconds(_ date: Date) -> Int { Int(pine: date.timeIntervalSince1970 * 1000) ?? 0 }

    static func part(_ name: String, milliseconds stamp: Int) -> PineRuntimeValue {
        let c = PineCalendar.components(milliseconds: stamp)
        switch name {
        case "year": return .int(c.year)
        case "month": return .int(c.month)
        case "dayofmonth": return .int(c.day)
        case "hour": return .int(c.hour)
        case "minute": return .int(c.minute)
        case "second": return .int(c.second)
        default: return .int(c.weekday)
        }
    }

    /// The bar length as a Pine multiplier and unit suffix (`"5"`, `"4"`+`""`, `"1D"`, `"30S"`).
    private static func period(_ seconds: Double) -> (multiplier: Int, suffix: String) {
        func rounded(_ unit: Double) -> Int { max(1, Int(pine: (seconds / unit).rounded()) ?? 1) }
        if seconds >= 28 * day { return (rounded(30 * day), "M") }
        if seconds >= 7 * day { return (rounded(7 * day), "W") }
        if seconds >= day { return (rounded(day), "D") }
        if seconds < 60 { return (Int(pine: seconds) ?? 0, "S") }
        return (Int(pine: (seconds / 60).rounded()) ?? 0, "")
    }

    /// `timeframe.*` for bars `seconds` apart; nil until the spacing is known.
    static func timeframe(_ name: String, barSeconds seconds: Double) -> PineRuntimeValue? {
        guard seconds > 0 else { return nil }
        let isMonthly = seconds >= 28 * day
        let isWeekly = !isMonthly && seconds >= 7 * day
        let isDaily = !isWeekly && !isMonthly && seconds >= day
        switch name {
        case "timeframe.period":
            let (multiplier, suffix) = period(seconds)
            return .string("\(multiplier)\(suffix)")
        case "timeframe.multiplier": return .int(period(seconds).multiplier)
        case "timeframe.isdaily": return .bool(isDaily)
        case "timeframe.isweekly": return .bool(isWeekly)
        case "timeframe.ismonthly": return .bool(isMonthly)
        case "timeframe.isseconds": return .bool(seconds < 60)
        case "timeframe.isminutes": return .bool(seconds >= 60 && seconds < day)
        case "timeframe.isintraday": return .bool(seconds < day)
        case "timeframe.isdwm": return .bool(seconds >= day)
        default: return nil
        }
    }
}
