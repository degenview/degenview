import Foundation

/// Presentation-only formatting for Paper Trading values. The engine and CSV export
/// deliberately keep their full `Decimal` precision.
///
/// Tables format hundreds of cells per refresh, so the `NumberFormatter`s are built once per
/// (style, locale, currency, precision) and reused. `NumberFormatter.string(from:)` is thread-safe;
/// the lock only guards the dictionary.
enum PaperTradingFormatter {
    static func money(
        _ value: Decimal,
        currency: PaperCurrency,
        locale: Locale = .current
    ) -> String {
        let formatter = cached("money|\(locale.identifier)|\(currency.rawValue)") {
            let formatter = NumberFormatter()
            formatter.locale = locale
            formatter.numberStyle = .currency
            formatter.currencyCode = currency.rawValue
            formatter.roundingMode = .halfEven
            formatter.usesGroupingSeparator = true
            return formatter
        }
        return formatter.string(from: value as NSDecimalNumber)
            ?? "\(currency.rawValue) \(value)"
    }

    /// Formats a ratio (`0.23349`) as a percentage (`23.35%`).
    static func percent(_ ratio: Decimal, locale: Locale = .current) -> String {
        let formatter = cached("percent|\(locale.identifier)") {
            let formatter = NumberFormatter()
            formatter.locale = locale
            formatter.numberStyle = .percent
            formatter.minimumFractionDigits = 2
            formatter.maximumFractionDigits = 2
            formatter.roundingMode = .halfEven
            return formatter
        }
        return formatter.string(from: ratio as NSDecimalNumber) ?? "—"
    }

    /// Like `percent`, with a leading `+` for gains: `+23.35%`, `-4.10%`, `0.00%`.
    static func signedPercent(_ ratio: Decimal, locale: Locale = .current) -> String {
        let formatted = percent(ratio, locale: locale)
        return ratio > 0 ? "+\(formatted)" : formatted
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
        let digits = min(16, max(0, -increment.exponent))
        let formatter = cached("decimal|\(locale.identifier)|\(digits)") {
            let formatter = NumberFormatter()
            formatter.locale = locale
            formatter.numberStyle = .decimal
            formatter.minimumFractionDigits = 0
            formatter.maximumFractionDigits = digits
            formatter.roundingMode = .halfEven
            formatter.usesGroupingSeparator = true
            return formatter
        }
        return formatter.string(from: value as NSDecimalNumber) ?? value.description
    }

    // MARK: Cache

    private static let lock = NSLock()
    nonisolated(unsafe) private static var formatters: [String: NumberFormatter] = [:]

    private static func cached(_ key: String, make: () -> NumberFormatter) -> NumberFormatter {
        lock.lock()
        defer { lock.unlock() }
        if let formatter = formatters[key] { return formatter }
        let formatter = make()
        formatters[key] = formatter
        return formatter
    }
}
