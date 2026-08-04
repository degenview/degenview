import Foundation

/// Date format selection based on the size of one candle.
enum TimeAxisFormatter {

    /// Pick a date format for a label that names a single candle of `interval` seconds.
    ///
    /// The finest unit shown matches the candle: an hourly candle reads "Mon 22:00", a
    /// daily one "Mon, Aug 3". Going finer than the candle prints digits the data doesn't
    /// resolve — the minutes of an hourly candle are always :00, and a label that ever
    /// said 22:37 would be naming a moment no candle stands for.
    static func format(for interval: TimeInterval) -> String {
        if interval < 86400 { return "EEE HH:mm" }      // sub-daily (1m … 4h)
        if interval < 604800 { return "EEE, MMM d" }    // daily
        // Split weekly from monthly at two weeks rather than at a month: calendar months
        // are 28–31 days, and a threshold at 30 would format February as a week.
        if interval < 1209600 { return "MMM d, yyyy" }  // weekly
        return "MMM yyyy"                               // monthly and coarser
    }

    /// A candle's open time, rendered at the granularity that candle resolves.
    ///
    /// Local time: the label answers "when was this candle", and the user reads it
    /// against their own clock — the newest hourly candle should say the hour it is now.
    static func string(_ date: Date, interval: TimeInterval) -> String {
        let format = format(for: interval)
        if formatter.dateFormat != format {
            formatter.dateFormat = format
        }
        return formatter.string(from: date)
    }

    /// How long a span covers, in the two coarsest units it needs: "1d 14h", "3h 20m",
    /// "45m". For the ruler's read-out, where the exact seconds never matter.
    ///
    /// Years and months are calendar approximations — a multi-year span on a monthly
    /// chart reads better as "3y 1mo" than as "1127d".
    static func duration(_ seconds: TimeInterval) -> String {
        durationFormatter.string(from: abs(seconds)) ?? ""
    }

    /// Built once. Crosshair labels are formatted on every mouse move, for every chart
    /// in the tab, and `DateFormatter` is expensive to create.
    private static let formatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        return formatter
    }()

    /// Same reasoning as `formatter`: the ruler reformats this on every mouse move
    /// while a rectangle is being drawn.
    private static let durationFormatter: DateComponentsFormatter = {
        let formatter = DateComponentsFormatter()
        formatter.allowedUnits = [.year, .month, .day, .hour, .minute]
        formatter.unitsStyle = .abbreviated
        formatter.maximumUnitCount = 2
        formatter.zeroFormattingBehavior = .dropAll
        return formatter
    }()
}
