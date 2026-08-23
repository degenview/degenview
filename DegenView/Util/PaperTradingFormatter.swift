import Foundation

/// Presentation-only formatting for Paper Trading values. The engine and CSV export
/// deliberately keep their full `Decimal` precision.
enum PaperTradingFormatter {
    static func money(
        _ value: Decimal,
        currency: PaperCurrency,
        locale: Locale = .current
    ) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .currency
        formatter.currencyCode = currency.rawValue
        formatter.roundingMode = .halfEven
        formatter.usesGroupingSeparator = true
        return formatter.string(from: value as NSDecimalNumber)
            ?? "\(currency.rawValue) \(value)"
    }

    /// Formats a ratio (`0.23349`) as a percentage (`23.35%`).
    static func percent(_ ratio: Decimal, locale: Locale = .current) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .percent
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 2
        formatter.roundingMode = .halfEven
        return formatter.string(from: ratio as NSDecimalNumber) ?? "—"
    }

    static func price(
        _ value: Decimal,
        instrument: PaperInstrument,
        locale: Locale = .current
    ) -> String {
        decimal(value, increment: instrument.tickSize, locale: locale)
    }

    static func quantity(
        _ value: Decimal,
        instrument: PaperInstrument,
        locale: Locale = .current
    ) -> String {
        decimal(value, increment: instrument.quantityIncrement, locale: locale)
    }

    static func signedMoney(
        _ value: Decimal,
        currency: PaperCurrency,
        locale: Locale = .current
    ) -> String {
        let formatted = money(value, currency: currency, locale: locale)
        return value > 0 ? "+\(formatted)" : formatted
    }

    private static func decimal(_ value: Decimal, increment: Decimal, locale: Locale) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 0
        formatter.maximumFractionDigits = min(16, max(0, -increment.exponent))
        formatter.roundingMode = .halfEven
        formatter.usesGroupingSeparator = true
        return formatter.string(from: value as NSDecimalNumber) ?? value.description
    }
}
