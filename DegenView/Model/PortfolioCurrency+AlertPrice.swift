import Foundation

extension PortfolioCurrency {
    /// A price as an alert states it: `$67,432.19`, `$1.2346`, `$0.00000278`.
    ///
    /// A converted quote carries a long fractional tail, and a micro-cap token needs its leading
    /// zeros plus a few digits to mean anything, so the precision follows the magnitude: two decimals
    /// from 1,000 up, two to four from one unit, and below one four significant figures (trailing
    /// zeros trimmed, never fewer than two decimals). `Decimal.description` prints none of this.
    func formatAlertPrice(_ value: Decimal, locale: Locale = .current) -> String {
        let digits = Self.alertFractionDigits(for: abs(value), wholeUnits: self == .JPY, minimum: self == .BTC ? 0 : nil)
        if self == .BTC {
            return "BTC " + value.formatted(.number.precision(.fractionLength(digits)).locale(locale))
        }
        return value.formatted(.currency(code: rawValue).precision(.fractionLength(digits)).locale(locale))
    }

    /// The fraction-digit window for a price of this size.
    static func alertFractionDigits(for magnitude: Decimal, wholeUnits: Bool, minimum: Int? = nil) -> ClosedRange<Int> {
        if wholeUnits && magnitude >= 1 { return 0...0 }
        let floorDigits = wholeUnits ? 0 : (minimum ?? 2)
        if magnitude >= 1000 { return max(floorDigits, 2)...max(floorDigits, 2) }
        if magnitude >= 1 { return floorDigits...max(floorDigits, 4) }
        if magnitude == 0 { return floorDigits...floorDigits }
        var leadingZeros = 0
        var scaled = magnitude
        while scaled < Decimal(sign: .plus, exponent: -1, significand: 1) && leadingZeros < maxAlertPriceDecimals {
            scaled *= 10
            leadingZeros += 1
        }
        return floorDigits...max(floorDigits, min(maxAlertPriceDecimals, leadingZeros + 4))
    }

    private static let maxAlertPriceDecimals = 12
}
