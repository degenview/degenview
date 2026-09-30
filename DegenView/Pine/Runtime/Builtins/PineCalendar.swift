import Foundation

/// Proleptic Gregorian calendar math in UTC, independent of the machine's locale or zone.
/// Days are counted from the Unix epoch using Howard Hinnant's civil-calendar algorithms.
enum PineCalendar {
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

    private static let millisecondsPerDay = 86_400_000
    /// Timestamps beyond this many milliseconds from the epoch do not fit an `Int` with room to
    /// spare; `make` refuses them rather than overflow.
    private static let maxMilliseconds = 9e18

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

    /// Milliseconds since the Unix epoch. Out-of-range months and days roll over, as in Pine.
    /// Nil when the result would not fit an `Int`.
    static func make(
        year: Int, month: Int, day: Int, hour: Int = 0, minute: Int = 0, second: Int = 0,
        offsetMinutes: Int = 0
    ) -> Int? {
        let monthIndex = month - 1
        let y = year + floorDiv(monthIndex, 12)
        let m = floorMod(monthIndex, 12) + 1
        let days = daysFromCivil(y, m, 1) + (day - 1)
        guard abs(Double(days) * Double(millisecondsPerDay)) < maxMilliseconds else { return nil }
        let seconds = ((days * 24 + hour) * 60 + minute - offsetMinutes) * 60 + second
        return seconds * 1000
    }

    static func components(milliseconds: Int) -> Components {
        let days = floorDiv(milliseconds, millisecondsPerDay)
        let secondsOfDay = floorMod(floorDiv(milliseconds, 1000), 86_400)
        let civil = civilFromDays(days)
        // 1970-01-01 was a Thursday; Sunday-based index 4.
        let weekday = floorMod(days + 4, 7) + 1
        return .init(
            year: civil.year, month: civil.month, day: civil.day, hour: secondsOfDay / 3600,
            minute: secondsOfDay % 3600 / 60, second: secondsOfDay % 60, weekday: weekday)
    }
}
