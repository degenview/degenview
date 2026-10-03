import Foundation

/// Reads a number a person typed into a Paper Trading field.
///
/// Accepts both `1,234.56` and `1.234,56`, a lone `1,5`, and stray spaces. A single separator is
/// ambiguous (`1,234` is one thousand or 1.234), so the locale decides, with a three-digit tail
/// read as grouping when the locale's decimal separator is the other character.
enum PaperDecimalInput {
    static func parse(_ text: String, locale: Locale = .current) -> Decimal? {
        let cleaned = text.filter { !$0.isWhitespace && $0 != "'" && $0 != "\u{2019}" }
        guard !cleaned.isEmpty else { return nil }
        let normalized = normalize(cleaned, localeDecimal: locale.decimalSeparator ?? ".")
        guard isPlainNumber(normalized) else { return nil }
        let padded = normalized.hasPrefix(".") ? "0" + normalized : normalized
        return Decimal(string: padded, locale: Locale(identifier: "en_US_POSIX"))
    }

    /// A value as it should sit in an input field: the locale's decimal separator, no grouping, so
    /// `parse` reads it back unchanged ("1,234" for 1.234 in a comma locale is never read as 1234).
    static func text(_ value: Decimal, locale: Locale = .current) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .decimal
        formatter.usesGroupingSeparator = false
        formatter.minimumFractionDigits = 0
        formatter.maximumFractionDigits = 16
        return formatter.string(from: value as NSDecimalNumber) ?? value.description
    }

    /// Rewrites `text` to use `.` as the only separator, with grouping removed.
    private static func normalize(_ text: String, localeDecimal: String) -> String {
        let commas = text.filter { $0 == "," }.count
        let dots = text.filter { $0 == "." }.count
        switch (commas, dots) {
        case (0, 0), (0, 1):
            // A lone dot is a decimal point unless the locale writes decimals with a comma and the
            // dot is followed by exactly three digits ("1.234" is then a thousands group).
            if dots == 1, localeDecimal == ",", digitsAfterLast(".", in: text) == 3, hasLeadingGroupDigits(text, ".") {
                return text.replacingOccurrences(of: ".", with: "")
            }
            return text
        case (1, 0):
            if localeDecimal != ",", digitsAfterLast(",", in: text) == 3, hasLeadingGroupDigits(text, ",") {
                return text.replacingOccurrences(of: ",", with: "")
            }
            return text.replacingOccurrences(of: ",", with: ".")
        case (_, 0):
            return text.replacingOccurrences(of: ",", with: "")
        case (0, _):
            return text.replacingOccurrences(of: ".", with: "")
        default:
            // Both present: whichever comes last is the decimal separator.
            let lastComma = text.lastIndex(of: ",")!
            let lastDot = text.lastIndex(of: ".")!
            if lastComma > lastDot {
                return text.replacingOccurrences(of: ".", with: "").replacingOccurrences(of: ",", with: ".")
            }
            return text.replacingOccurrences(of: ",", with: "")
        }
    }

    private static func digitsAfterLast(_ separator: Character, in text: String) -> Int {
        guard let index = text.lastIndex(of: separator) else { return 0 }
        return text.distance(from: index, to: text.endIndex) - 1
    }

    /// "1,234" groups; "0,500" and ",500" do not — a leading zero is a fraction.
    private static func hasLeadingGroupDigits(_ text: String, _ separator: Character) -> Bool {
        guard let index = text.firstIndex(of: separator) else { return false }
        let head = text[text.startIndex..<index]
        return (1...3).contains(head.count) && head.first != "0" && head.allSatisfy(\.isNumber)
    }

    private static func isPlainNumber(_ text: String) -> Bool {
        var body = Substring(text)
        if body.first == "-" { body = body.dropFirst() }
        guard !body.isEmpty, body.filter({ $0 == "." }).count <= 1 else { return false }
        return body.contains(where: \.isNumber) && body.allSatisfy { $0.isNumber || $0 == "." }
    }
}
