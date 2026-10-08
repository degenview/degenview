import Foundation

/// What an import did, shown once so nothing is dropped silently.
struct WatchlistImportReport: Equatable, Sendable {
    struct Skipped: Equatable, Sendable {
        /// 1-based line (or comma-separated item) in the pasted text.
        let line: Int
        let text: String
        let reason: String
    }

    var added = 0
    var duplicates = 0
    var sectionsAdded = 0
    var skipped: [Skipped] = []

    var summary: String {
        var parts = ["Added \(added) symbol\(added == 1 ? "" : "s")"]
        if sectionsAdded > 0 { parts.append("\(sectionsAdded) section\(sectionsAdded == 1 ? "" : "s")") }
        if duplicates > 0 { parts.append("\(duplicates) already in the list") }
        if !skipped.isEmpty { parts.append("\(skipped.count) skipped") }
        return parts.joined(separator: ", ")
    }
}
