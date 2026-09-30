import Foundation

/// Pine's `timestamp()`: argument handling and date-string parsing on top of `PineCalendar`.
/// Everything is UTC unless a time zone is named, so results never depend on the machine's
/// locale or zone.
enum PineTimestamp {
    private static let monthNames = [
        "january", "february", "march", "april", "may", "june", "july", "august", "september",
        "october", "november", "december",
    ]

    /// Longest abbreviation people write for a month: "sept".
    private static let longestAbbreviation = 4

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
        return PineCalendar.make(
            year: year, month: month, day: day, hour: fields[3] ?? 0, minute: fields[4] ?? 0,
            second: fields[5] ?? 0, offsetMinutes: zone ?? 0)
    }

    private static func integer(_ value: PineRuntimeValue) -> Int? {
        guard let n = value.number, abs(n) < 1e12 else { return nil }
        return Int(pine: n)
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
            } else if let index = monthIndex(of: lower) {
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
        return PineCalendar.make(
            year: year, month: month, day: day, hour: time.hour, minute: time.minute,
            second: time.second, offsetMinutes: zone)
    }

    /// Index of the month a word names: its full name, or an abbreviation of at least three
    /// letters ("jan", "sept"). "mayhem" is not May.
    private static func monthIndex(of word: String) -> Int? {
        let trimmed = word.trimmingCharacters(in: CharacterSet(charactersIn: "."))
        if let exact = monthNames.firstIndex(of: trimmed) { return exact }
        guard (3...longestAbbreviation).contains(trimmed.count) else { return nil }
        return monthNames.firstIndex { $0.hasPrefix(trimmed) }
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
