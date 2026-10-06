import Foundation

/// Everything the transaction editor needs to know about what is typed into it — parsed values,
/// the price/total relationship, the summary figures and which fields are wrong — kept out of the
/// view so it can be tested.
///
/// Price and total describe the same number twice, so only one of them is typed at a time: the one
/// edited last is the source of truth and the other is derived from it and the quantity.
/// `PortfolioAccountingEngine` stays the authority on what a ledger accepts; this only warns
/// before saving.
struct TransactionEditorDraft: Equatable {
    enum Field: Hashable {
        case quantity, price, total, fee
    }

    /// The five groups the type selector offers; each holds one or more `PortfolioTransactionType`s.
    enum Category: String, CaseIterable, Identifiable {
        case buy = "Buy"
        case sell = "Sell"
        case transfer = "Transfer"
        case income = "Income"
        case other = "Other"

        var id: String { rawValue }

        var systemImage: String {
            switch self {
            case .buy: "plus.circle"
            case .sell: "minus.circle"
            case .transfer: "arrow.left.arrow.right"
            case .income: "gift"
            case .other: "ellipsis.circle"
            }
        }

        var types: [PortfolioTransactionType] {
            switch self {
            case .buy: [.buy]
            case .sell: [.sell]
            case .transfer: [.transferIn, .transferOut]
            case .income: [.reward, .stakingReward, .airdrop, .mining, .interest]
            case .other: [.fee, .adjustment]
            }
        }

        init(_ type: PortfolioTransactionType) {
            self = Self.allCases.first { $0.types.contains(type) } ?? .other
        }
    }

    /// Which of the two linked fields the user typed in last.
    enum PricedField: Hashable {
        case price, total
    }

    struct Summary: Equatable {
        var value: Decimal
        var fee: Decimal
        /// "Total cost" or "Net proceeds" with the figure; nil for types where it has no meaning.
        var net: (label: String, amount: Decimal)?

        static func == (lhs: Self, rhs: Self) -> Bool {
            lhs.value == rhs.value && lhs.fee == rhs.fee && lhs.net?.label == rhs.net?.label
                && lhs.net?.amount == rhs.net?.amount
        }
    }

    var type: PortfolioTransactionType
    var quantityText: String
    var feeText: String
    var date: Date
    var notes: String
    var locale: Locale = .current
    private(set) var lastEdited: PricedField = .price
    private var priceText: String
    private var totalText = ""
    private let currency: PortfolioCurrency

    init(transaction: PortfolioTransaction, locale: Locale = .current) {
        self.locale = locale
        currency = transaction.priceCurrency
        type = transaction.type
        // Stored values keep every digit they have; only the grouping and a two-decimal floor on
        // prices are added, so opening a transaction and saving it unchanged cannot alter it.
        quantityText =
            transaction.quantity == 0 ? "" : Self.exact(transaction.quantity, minimumFraction: 0, locale: locale)
        priceText = transaction.price.map { Self.exact($0, minimumFraction: 2, locale: locale) } ?? ""
        feeText = transaction.fee == 0 ? "" : Self.exact(transaction.fee, minimumFraction: 0, locale: locale)
        date = transaction.timestamp
        notes = transaction.notes
    }

    // MARK: Type

    var category: Category { Category(type) }

    /// Moves to `category`, keeping the current type when it already belongs there.
    mutating func select(_ category: Category) {
        guard !category.types.contains(type), let first = category.types.first else { return }
        type = first
    }

    /// Transfer-outs, fees and adjustments carry no price.
    var usesPrice: Bool { ![.transferOut, .fee, .adjustment].contains(type) }
    /// The accounting engine rejects a buy or sell without one.
    var requiresPrice: Bool { type == .buy || type == .sell }

    // MARK: Typed values

    var quantity: Decimal? { parse(quantityText) }
    var fee: Decimal? { feeText.trimmingCharacters(in: .whitespaces).isEmpty ? 0 : parse(feeText) }

    /// The price per unit: typed, or derived from the typed total.
    var price: Decimal? {
        switch lastEdited {
        case .price:
            return parse(priceText)
        case .total:
            guard let total = parse(totalText), let quantity, quantity > 0 else { return nil }
            return Decimal.rounded(total / quantity, scale: 8)
        }
    }

    /// The total: typed, or the quantity at the price.
    var total: Decimal? {
        switch lastEdited {
        case .total:
            return parse(totalText)
        case .price:
            guard let quantity, quantity > 0, let price = parse(priceText) else { return nil }
            return Decimal.rounded(quantity * price, scale: 8)
        }
    }

    /// What the price field shows: its own text while it is the source, else the derived figure.
    var displayedPrice: String { lastEdited == .price ? priceText : price.map(text(forMarketPrice:)) ?? "" }
    var displayedTotal: String { lastEdited == .total ? totalText : total.map(text(forTotal:)) ?? "" }

    mutating func setPrice(_ text: String) {
        guard text != displayedPrice else { return }
        priceText = text
        lastEdited = .price
    }

    mutating func setTotal(_ text: String) {
        guard text != displayedTotal else { return }
        totalText = text
        lastEdited = .total
    }

    /// A price as it should read in the price field: grouped, with the precision of the price's
    /// size — two decimals from 1,000 up, up to four from one unit, four significant figures below
    /// it, like every other price in the app.
    func text(forMarketPrice value: Decimal) -> String {
        let digits = PortfolioCurrency.alertFractionDigits(
            for: abs(value), wholeUnits: currency == .JPY, minimum: currency == .BTC ? 0 : nil)
        return Self.format(value, fraction: digits, locale: locale)
    }

    /// A derived total, in the currency's own precision: cents, whole yen, satoshis.
    func text(forTotal value: Decimal) -> String {
        let digits: ClosedRange<Int>
        switch currency {
        case .JPY: digits = 0...0
        case .BTC: digits = 0...8
        default: digits = 2...2
        }
        return Self.format(value, fraction: digits, locale: locale)
    }

    /// Fills the price with `value` — but only while the user has not entered a price or a total,
    /// so a market price arriving late never overwrites what they typed.
    mutating func prefillPrice(_ value: Decimal) {
        guard value > 0, lastEdited == .price, priceText.trimmingCharacters(in: .whitespaces).isEmpty else {
            return
        }
        priceText = text(forMarketPrice: value)
    }

    // MARK: Validation

    /// Fields with something wrong. An empty field is missing, not wrong, so a fresh form never
    /// opens red.
    var issues: [Field: String] {
        var result: [Field: String] = [:]
        if let message = numberIssue(quantityText, allowsZero: false) { result[.quantity] = message }
        if usesPrice {
            if lastEdited == .price, let message = numberIssue(priceText, allowsZero: true) {
                result[.price] = message
            }
            if lastEdited == .total, let message = numberIssue(totalText, allowsZero: true) {
                result[.total] = message
            }
        }
        if let message = numberIssue(feeText, allowsZero: true) { result[.fee] = message }
        return result
    }

    var canSubmit: Bool {
        guard let quantity, quantity > 0, fee != nil, issues.isEmpty else { return false }
        return !(usesPrice && requiresPrice) || price != nil
    }

    // MARK: Summary

    /// The cost or proceeds of the transaction; nil until there is a quantity and a price.
    var summary: Summary? {
        guard usesPrice, let total, let fee else { return nil }
        let net: (label: String, amount: Decimal)?
        switch type {
        case .buy: net = ("Total cost", total + fee)
        case .transferIn: net = ("Total cost", total + fee)
        case .sell: net = ("Net proceeds", total - fee)
        default: net = nil
        }
        return Summary(value: total, fee: fee, net: net)
    }

    // MARK: Output

    /// `base` with the typed values applied; nil while the draft cannot be submitted. A price the
    /// chosen type does not use is dropped rather than kept from an earlier choice.
    func makeTransaction(from base: PortfolioTransaction, now: Date = Date()) -> PortfolioTransaction? {
        guard canSubmit, let quantity, let fee else { return nil }
        var transaction = base
        transaction.type = type
        transaction.quantity = quantity
        transaction.price = usesPrice ? price : nil
        transaction.fee = fee
        transaction.timestamp = date
        transaction.notes = notes
        transaction.updatedAt = now
        return transaction
    }

    // MARK: Helpers

    private func parse(_ text: String) -> Decimal? { PaperDecimalInput.parse(text, locale: locale) }

    /// Every digit of `value`, grouped, with at least `minimumFraction` decimals.
    private static func exact(_ value: Decimal, minimumFraction: Int, locale: Locale) -> String {
        format(value, fraction: minimumFraction...16, locale: locale)
    }

    /// Grouped in the locale's own separators; `parse` reads the result back unchanged.
    private static func format(_ value: Decimal, fraction: ClosedRange<Int>, locale: Locale) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .decimal
        formatter.usesGroupingSeparator = true
        formatter.minimumFractionDigits = fraction.lowerBound
        formatter.maximumFractionDigits = fraction.upperBound
        return formatter.string(from: value as NSDecimalNumber) ?? value.description
    }

    private func numberIssue(_ text: String, allowsZero: Bool) -> String? {
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        guard let value = parse(text) else { return "Enter a number." }
        if value < 0 { return "Can't be negative." }
        if value == 0 && !allowsZero { return "Must be greater than zero." }
        return nil
    }
}
