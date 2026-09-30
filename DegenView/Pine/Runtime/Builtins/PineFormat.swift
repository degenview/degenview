import Foundation

/// `str.tostring` formatting. Patterns support `#`/`0` fraction digits (`"#.##"`, `"0.00"`)
/// and `format.mintick`/`format.percent`.
enum PineFormat {
    /// Fraction digits used for `format.mintick` when the mintick is unusable.
    private static let fallbackDecimals = 2
    private static let defaultMaxDigits = 8

    static func format(_ value: PineRuntimeValue, _ pattern: String?, mintick: Double) -> String {
        switch value {
        case .string(let s): return s
        case .bool(let b): return b ? "true" : "false"
        case .int(let i) where pattern == nil: return String(i)
        case .int, .float: break
        default: return "NaN"
        }
        guard let x = value.number, x.isFinite else { return "NaN" }
        let style = digitStyle(pattern, mintick: mintick)
        var text = String(format: "%.\(style.maxDigits)f", x)
        if style.maxDigits > style.minDigits, text.contains(".") {
            var trimmable = style.maxDigits - style.minDigits
            while trimmable > 0, text.hasSuffix("0") {
                text.removeLast()
                trimmable -= 1
            }
            if text.hasSuffix(".") { text.removeLast() }
        }
        if text == "-0" { text = "0" }
        return text + style.suffix
    }

    private static func digitStyle(
        _ pattern: String?, mintick: Double
    ) -> (minDigits: Int, maxDigits: Int, suffix: String) {
        switch pattern {
        case nil, "format.volume", "format.inherit": return (0, defaultMaxDigits, "")
        case "format.mintick":
            let decimals =
                mintick > 0 && mintick.isFinite
                ? max(0, Int(pine: (-log10(mintick)).rounded(.up)) ?? fallbackDecimals) : fallbackDecimals
            return (decimals, decimals, "")
        case "format.percent": return (2, 2, "%")
        case let pattern?:
            guard let dot = pattern.firstIndex(of: ".") else { return (0, 0, "") }
            let fraction = pattern[pattern.index(after: dot)...]
            return (
                fraction.filter { $0 == "0" }.count, fraction.filter { $0 == "0" || $0 == "#" }.count, ""
            )
        }
    }
}
