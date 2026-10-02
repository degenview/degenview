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
        "timeframe.isintraday", "timeframe.isdwm", "timeframe.isticks",
    ]

    private static let day = 86_400.0

    static func milliseconds(_ date: Date) -> Int { Int(pine: date.timeIntervalSince1970 * 1000) ?? 0 }

    /// The calendar fields of `stamp` in `zone`, UTC when none is given.
    static func components(milliseconds stamp: Int, zone: TimeZone? = nil) -> PineCalendar.Components {
        guard let zone else { return PineCalendar.components(milliseconds: stamp) }
        let offset = zone.secondsFromGMT(for: Date(timeIntervalSince1970: Double(stamp) / 1000))
        return PineCalendar.components(milliseconds: stamp + offset * 1000)
    }

    static func part(_ name: String, milliseconds stamp: Int, zone: TimeZone? = nil) -> PineRuntimeValue {
        let c = components(milliseconds: stamp, zone: zone)
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

    /// Length in seconds of a timeframe string: `"30S"`, `"5"` and `"60"` (minutes), `"1D"`, `"2W"`,
    /// `"3M"`, or a bare unit such as `"D"`. A month counts as 30 days, as `period(_:)` does. Nil when malformed.
    static func seconds(ofTimeframe text: String) -> Double? {
        let digits = text.prefix { $0.isASCII && $0.isNumber }
        let unit = text.dropFirst(digits.count).uppercased()
        guard let multiplier = digits.isEmpty ? (unit.isEmpty ? nil : 1) : Double(digits), multiplier > 0
        else { return nil }
        switch unit {
        case "": return multiplier * 60
        case "S": return multiplier
        case "D": return multiplier * day
        case "W": return multiplier * 7 * day
        case "M": return multiplier * 30 * day
        default: return nil
        }
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
        // Bars here are always time based.
        case "timeframe.isticks": return .bool(false)
        default: return nil
        }
    }

    /// `str.format_time`: `format` is a Unicode date pattern (the letters Pine and Java share: `yyyy`, `MM`,
    /// `MMM`, `dd`, `HH`, `hh`, `mm`, `ss`, `SSS`, `a`, `EEE`, `Z`…), `zone` an IANA name, `UTC`, or
    /// `UTC+3` / `GMT-05:30`. Without a zone it is UTC, which is what the engine reports as the exchange zone.
    static func formatted(milliseconds: Int, format: String?, zone: String?) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.timeZone = timeZone(named: zone)
        formatter.dateFormat = format ?? "yyyy-MM-dd'T'HH:mm:ssZ"
        return formatter.string(from: Date(timeIntervalSince1970: Double(milliseconds) / 1000))
    }

    static func timeZone(named name: String?) -> TimeZone {
        guard let name, !name.isEmpty else { return TimeZone(secondsFromGMT: 0) ?? .current }
        if let zone = TimeZone(identifier: name) { return zone }
        let parts = name.uppercased().replacingOccurrences(of: " ", with: "")
        for prefix in ["UTC", "GMT"] where parts.hasPrefix(prefix) {
            let offset = parts.dropFirst(prefix.count)
            guard let sign = offset.first, sign == "+" || sign == "-" else { break }
            let digits = offset.dropFirst().split(separator: ":", omittingEmptySubsequences: false)
            guard let hours = digits.first.flatMap({ Int($0) }) else { break }
            let minutes = digits.count > 1 ? Int(digits[1]) ?? 0 : 0
            let seconds = (hours * 3600 + minutes * 60) * (sign == "-" ? -1 : 1)
            if let zone = TimeZone(secondsFromGMT: seconds) { return zone }
        }
        return TimeZone(secondsFromGMT: 0) ?? .current
    }
}
