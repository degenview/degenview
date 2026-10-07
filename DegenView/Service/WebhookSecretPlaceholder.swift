import Foundation

/// The `{{secret}}` token users put in a webhook's URL or header values. Tolerant of spacing and
/// case (`{{ Secret }}`), because it is typed by people, not generated.
enum WebhookSecretPlaceholder {
    /// What the editor inserts.
    static let token = "{{secret}}"

    private static let regex: NSRegularExpression = {
        do {
            return try NSRegularExpression(pattern: #"\{\{\s*secret\s*\}\}"#, options: [.caseInsensitive])
        } catch {
            preconditionFailure("invalid secret placeholder pattern: \(error)")
        }
    }()

    static func contains(_ text: String) -> Bool {
        regex.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
    }

    /// Replaces every placeholder with `replacement`, literally: `$` and `\` in the replacement mean
    /// nothing special.
    static func replacing(in text: String, with replacement: String) -> String {
        let matches = regex.matches(in: text, range: NSRange(text.startIndex..., in: text))
        guard !matches.isEmpty else { return text }
        var result = ""
        var cursor = text.startIndex
        for match in matches {
            guard let range = Range(match.range, in: text) else { continue }
            result += text[cursor..<range.lowerBound]
            result += replacement
            cursor = range.upperBound
        }
        result += text[cursor...]
        return result
    }
}
