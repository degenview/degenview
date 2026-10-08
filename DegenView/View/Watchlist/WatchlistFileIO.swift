import AppKit
import UniformTypeIdentifiers

/// Open and save panels for watchlist text files.
@MainActor
enum WatchlistFileIO {
    static func chooseTextFile() -> String? {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.plainText, .commaSeparatedText, .text]
        panel.allowsMultipleSelection = false
        panel.message = "Choose a text or CSV file of symbols"
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return try? String(contentsOf: url, encoding: .utf8)
    }

    /// Returns an error message if the file could not be written; nil on success or cancel.
    static func save(_ text: String, suggestedName: String) -> String? {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.plainText]
        panel.nameFieldStringValue = "\(suggestedName).txt"
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        do {
            try text.write(to: url, atomically: true, encoding: .utf8)
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    static func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}
