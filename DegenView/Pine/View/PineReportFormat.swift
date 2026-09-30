import Foundation

/// Number formatting for the strategy report. Values are plain `Double`s from the
/// interpreter, so this is separate from the `Decimal`-based paper-trading formatter.
enum PineReportFormat {
    private static let minusSign = "−"

    static func money(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(2)).grouping(.automatic))
    }

    static func signedMoney(_ value: Double) -> String {
        (value >= 0 ? "+" : minusSign) + money(abs(value))
    }

    static func percent(_ fraction: Double, signed: Bool = false) -> String {
        let text = (abs(fraction) * 100).formatted(.number.precision(.fractionLength(2))) + "%"
        guard signed else { return text }
        return (fraction >= 0 ? "+" : minusSign) + text
    }

    static func price(_ value: Double) -> String {
        value.formatted(.number.precision(.significantDigits(1...7)))
    }

    static func ratio(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(2)))
    }
}
