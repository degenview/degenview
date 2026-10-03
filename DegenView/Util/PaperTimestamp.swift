import Foundation

/// The one timestamp style in the panel's tables: month, day and time to the second, in the user's locale.
enum PaperTimestamp {
    static func format(_ date: Date) -> String {
        date.formatted(.dateTime.month(.abbreviated).day().hour().minute().second())
    }
}
