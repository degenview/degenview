import Foundation

/// Calendar math for Pine's `timestamp()` and the `year`/`month`/… built-ins. Everything is
/// proleptic Gregorian UTC unless a time zone is named, so results never depend on the
/// machine's locale or zone.
enum PineTimestamp {
    struct Components: Equatable {
        var year: Int
        var month: Int
        var day: Int
        var hour: Int
        var minute: Int
        var second: Int
        /// Pine's `dayofweek`: Sunday is 1 … Saturday is 7.
        var weekday: Int
    }

    private static let monthNames = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]

    // MARK: - Civil calendar (Howard Hinnant's algorithms)

    static func daysFromCivil(_ year: Int, _ month: Int, _ day: Int) -> Int {
        let y = month <= 2 ? year - 1 : year
        let era = (y >= 0 ? y : y - 399) / 400
        let yoe = y - era * 400
        let mp = (month + 9) % 12
        let doy = (153 * mp + 2) / 5 + day - 1
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
        return era * 146_097 + doe - 719_468
    }

    static func civilFromDays(_ days: Int) -> (year: Int, month: Int, day: Int) {
        let z = days + 719_468
        let era = (z >= 0 ? z : z - 146_096) / 146_097
        let doe = z - era * 146_097
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
        let mp = (5 * doy + 2) / 153
        let day = doy - (153 * mp + 2) / 5 + 1
        let month = mp < 10 ? mp + 3 : mp - 9
        let year = yoe + era * 400
        return (month <= 2 ? year + 1 : year, month, day)
    }

    private static func floorDiv(_ a: Int, _ b: Int) -> Int {
        let q = a / b
        return (a % b != 0 && (a < 0) != (b < 0)) ? q - 1 : q
    }

    private static func floorMod(_ a: Int, _ b: Int) -> Int { a - floorDiv(a, b) * b }

    // MARK: - Construction and decomposition

    /// Milliseconds since the Unix epoch. Out-of-range months and days roll over, as in Pine.
    static func make(
        year: Int, month: Int, day: Int, hour: Int = 0, minute: Int = 0, second: Int = 0,
        offsetMinutes: Int = 0
    ) -> Int {
        let monthIndex = month - 1
        let y = year + floorDiv(monthIndex, 12)
        let m = floorMod(monthIndex, 12) + 1
        let days = daysFromCivil(y, m, 1) + (day - 1)
        let seconds = ((days * 24 + hour) * 60 + minute - offsetMinutes) * 60 + second
        return seconds * 1000
    }

    static func components(milliseconds: Int) -> Components {
        let days = floorDiv(milliseconds, 86_400_000)
        let secondsOfDay = floorMod(floorDiv(milliseconds, 1000), 86_400)
        let civil = civilFromDays(days)
        // 1970-01-01 was a Thursday; Sunday-based index 4.
        let weekday = floorMod(days + 4, 7) + 1
        return .init(
            year: civil.year, month: civil.month, day: civil.day, hour: secondsOfDay / 3600,
            minute: secondsOfDay % 3600 / 60, second: secondsOfDay % 60, weekday: weekday)
    }

    // MARK: - Pine entry point

    /// `timestamp(dateString)`, `timestamp(y, m, d[, h, mi, s])` and
    /// `timestamp(timezone, y, m, d[, h, mi, s])`, positional or named. Returns nil for `na`
    /// or unparseable input.
    static func evaluate(
        positional: [PineRuntimeValue], named: [String: PineRuntimeValue] = [:]
    ) -> Int? {
        let order = ["year", "month", "day", "hour", "minute", "second"]
        var values = positional
        var zone: Int?
        if case .string(let text)? = values.first {
            if values.count == 1 && named.isEmpty { return parse(text) }
            zone = offset(of: text)
            if zone == nil { return nil }
            values.removeFirst()
        }
        var fields: [Int?] = Array(repeating: nil, count: order.count)
        for (i, value) in values.prefix(order.count).enumerated() { fields[i] = integer(value) }
        for (name, value) in named {
            if let i = order.firstIndex(of: name) { fields[i] = integer(value) }
        }
        guard let year = fields[0], let month = fields[1], let day = fields[2] else { return nil }
        // An `na` argument makes the whole timestamp `na`; omitted time fields default to 0.
        if values.contains(where: { $0 == .na }) { return nil }
        return make(
            year: year, month: month, day: day, hour: fields[3] ?? 0, minute: fields[4] ?? 0,
            second: fields[5] ?? 0, offsetMinutes: zone ?? 0)
    }

    private static func integer(_ value: PineRuntimeValue) -> Int? {
        guard let n = value.number, n.isFinite, abs(n) < 1e12 else { return nil }
        return Int(n)
    }

    // MARK: - Date strings

    /// Accepts ISO (`2018-01-01`, `2018-01-01T12:30:00Z`) and the `01 Jan 2018 00:00 +0000`
    /// family (any order of day/month-name/year, optional time, optional zone).
    static func parse(_ text: String) -> Int? {
        var year: Int?
        var month: Int?
        var day: Int?
        var time = (hour: 0, minute: 0, second: 0)
        var zone = 0

        let separators = CharacterSet(charactersIn: " ,\t")
        var tokens = text.components(separatedBy: separators).filter { !$0.isEmpty }
        if tokens.count == 1, let t = tokens.first, t.contains("T"), t.first?.isNumber == true {
            let parts = t.split(separator: "T", maxSplits: 1).map(String.init)
            tokens = parts
        }
        var numbers: [Int] = []
        for token in tokens {
            let lower = token.lowercased()
            if let z = zoneToken(lower, token) {
                zone = z
            } else if lower.contains(":") {
                // `12:30[:45]`, possibly with a glued zone (`12:30:00Z`, `12:30+02:00`).
                var clock = lower
                if let index = clock.firstIndex(where: { $0 == "z" || $0 == "+" || $0 == "-" }) {
                    zone = offset(of: String(clock[index...])) ?? 0
                    clock = String(clock[..<index])
                }
                let parts = clock.split(separator: ":").compactMap { Int($0) }
                guard parts.count >= 2 else { return nil }
                time = (parts[0], parts[1], parts.count > 2 ? parts[2] : 0)
            } else if let index = monthNames.firstIndex(where: { lower.hasPrefix($0) }),
                lower.first?.isLetter == true
            {
                month = index + 1
            } else if lower.contains("-"), lower.first?.isNumber == true {
                let parts = lower.split(separator: "-").compactMap { Int($0) }
                guard parts.count == 3 else { return nil }
                (year, month, day) = (parts[0], parts[1], parts[2])
            } else if let n = Int(lower) {
                numbers.append(n)
            } else {
                return nil
            }
        }
        for n in numbers {
            if n > 31 || (day != nil && year == nil) {
                year = year ?? n
            } else if day == nil {
                day = n
            } else if year == nil {
                year = n
            }
        }
        guard let year, let month, let day else { return nil }
        return make(
            year: year, month: month, day: day, hour: time.hour, minute: time.minute,
            second: time.second, offsetMinutes: zone)
    }

    private static func zoneToken(_ lower: String, _ token: String) -> Int? {
        let looksLikeZone =
            lower == "z" || lower.hasPrefix("gmt") || lower.hasPrefix("utc")
            || ((lower.hasPrefix("+") || lower.hasPrefix("-")) && lower.count >= 2)
        return looksLikeZone ? offset(of: token) : nil
    }

    /// Minutes east of UTC for `Z`, `+0200`, `-05:00`, `GMT+2`, `UTC-3`, `UTC`.
    static func offset(of text: String) -> Int? {
        var s = text.uppercased()
        if s == "Z" || s == "UTC" || s == "GMT" { return 0 }
        if s.hasPrefix("GMT") { s.removeFirst(3) } else if s.hasPrefix("UTC") { s.removeFirst(3) }
        guard let sign = s.first, sign == "+" || sign == "-" else { return nil }
        let digits = s.dropFirst().replacingOccurrences(of: ":", with: "")
        guard !digits.isEmpty, digits.allSatisfy(\.isNumber), digits.count <= 4 else { return nil }
        let value = Int(digits) ?? 0
        let minutes: Int
        if digits.count <= 2 { minutes = value * 60 } else { minutes = value / 100 * 60 + value % 100 }
        return sign == "-" ? -minutes : minutes
    }
}
