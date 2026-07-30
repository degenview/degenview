import Foundation

/// Date format selection based on visible time span.
enum TimeAxisFormatter {

    /// Pick a sensible date format for the given visible time span (in seconds).
    static func format(for span: TimeInterval) -> String {
        if span < 7200 { return "HH:mm" }            // < 2 hours
        if span < 172800 { return "EEE HH:mm" }       // < 2 days
        if span < 2592000 { return "MMM d" }          // < ~1 month
        return "MMM yy"                               // months–years
    }
}
