import SwiftUI

/// How the portfolio screens write amounts: one reporting currency, one privacy switch.
///
/// Amounts mask when privacy mode is on; percentages and quantities of a position's *share*
/// (allocation, return) stay readable, matching the Holdings table.
struct PortfolioValueFormatter {
    let currency: PortfolioCurrency
    let privacy: Bool

    static let mask = "••••••••"

    /// A currency amount, masked in privacy mode.
    func money(_ value: Decimal) -> String {
        PortfolioPrivacy.sensitive(currency.format(value), enabled: privacy)
    }

    /// A per-unit price (cost, quote), masked in privacy mode. Keeps the decimals of sub-unit prices.
    func price(_ value: Decimal) -> String {
        PortfolioPrivacy.sensitive(currency.formatPrice(value), enabled: privacy)
    }

    /// "+$1,234.00" / "−$12.00"; zero carries no sign.
    func signedMoney(_ value: Decimal) -> String {
        guard !privacy else { return Self.mask }
        let text = currency.format(abs(value))
        if value > 0 { return "+" + text }
        if value < 0 { return "−" + text }
        return text
    }

    func percent(_ value: Decimal, digits: Int = 2) -> String {
        value.formatted(.percent.precision(.fractionLength(digits)))
    }

    /// "+12.34%" / "−3.10%".
    func signedPercent(_ value: Decimal, digits: Int = 2) -> String {
        let text = abs(value).formatted(.percent.precision(.fractionLength(digits)))
        if value > 0 { return "+" + text }
        if value < 0 { return "−" + text }
        return text
    }

    enum QuantitySign { case none, plus, minus }

    /// An asset quantity, masked in privacy mode. `sign` marks inflows and outflows ("+0.5", "−0.2").
    func quantity(_ value: Decimal, sign: QuantitySign = .none) -> String {
        guard !privacy else { return Self.mask }
        let text = value.portfolioQuantity
        switch sign {
        case .none: return text
        case .plus: return "+" + text
        case .minus: return "−" + text
        }
    }

    /// Green for gains, red for losses, nil (so: primary) for zero or unknown, and never
    /// colored while values are hidden — the color would give the sign away.
    func tone(_ value: Decimal?) -> Color? {
        guard !privacy, let value, value != 0 else { return nil }
        return value > 0 ? .green : .red
    }
}
