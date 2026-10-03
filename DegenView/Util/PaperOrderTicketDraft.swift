import Foundation

/// Everything the order ticket needs to know about what is typed into it — parsed values, the
/// figures in the summary, and which fields are wrong — kept out of the view so it can be tested.
///
/// The margin and tick rules mirror `PaperTradingEngine` (`requiredMargin`, `validate`): margin is
/// charged only for the part of an order that adds exposure, and quantities and prices must sit on
/// the instrument's increments. The engine stays the authority; this only warns before submitting.
struct PaperOrderTicketDraft {
    enum Field: Hashable {
        case quantity, limitPrice, stopPrice, takeProfit, stopLoss
    }

    // Market context
    let instrument: PaperInstrument
    var bid: Decimal?
    var ask: Decimal?
    var last: Decimal?
    var availableFunds: Decimal?
    var leverage: Decimal = 1
    var commission: PaperCommissionConfiguration = .none
    /// The open position in this instrument: positive long, negative short, zero none.
    var existingSignedQuantity: Decimal = 0
    var locale: Locale = .current

    // What is typed
    var side: PaperOrderSide = .buy
    var type: PaperOrderType = .market
    var timeInForce: PaperTimeInForce = .goodTilCanceled
    var quantityText = ""
    var limitPriceText = ""
    var stopPriceText = ""
    var takeProfitText = ""
    var stopLossText = ""

    // MARK: Parsed values

    var quantity: Decimal? { PaperDecimalInput.parse(quantityText, locale: locale) }
    var limitPrice: Decimal? { PaperDecimalInput.parse(limitPriceText, locale: locale) }
    var stopPrice: Decimal? { PaperDecimalInput.parse(stopPriceText, locale: locale) }
    var takeProfit: Decimal? { PaperDecimalInput.parse(takeProfitText, locale: locale) }
    var stopLoss: Decimal? { PaperDecimalInput.parse(stopLossText, locale: locale) }

    var usesLimitPrice: Bool { type == .limit || type == .stopLimit }
    var usesStopPrice: Bool { type == .stop || type == .stopLimit }

    // MARK: Pricing

    /// What a market order would fill at: the ask to buy, the bid to sell, else the last trade.
    var marketPrice: Decimal? { (side == .buy ? ask : bid) ?? last }

    /// The price the order is valued at: its own limit or stop, else the market.
    var valuationPrice: Decimal? {
        switch type {
        case .market: marketPrice
        case .limit, .stopLimit: limitPrice
        case .stop: stopPrice
        }
    }

    var spread: Decimal? {
        guard let bid, let ask, ask >= bid else { return nil }
        return ask - bid
    }

    var notional: Decimal? {
        guard let quantity, let price = valuationPrice else { return nil }
        return quantity * price * instrument.contractMultiplier
    }

    /// The part of this order that closes the opposite open position.
    var reducibleQuantity: Decimal {
        let reduces = (existingSignedQuantity > 0 && side == .sell) || (existingSignedQuantity < 0 && side == .buy)
        return reduces ? abs(existingSignedQuantity) : 0
    }

    /// Larger than the open position on the other side: the order closes it and opens the rest.
    var flipsPosition: Bool {
        guard let quantity, reducibleQuantity > 0 else { return false }
        return quantity > reducibleQuantity
    }

    var requiredMargin: Decimal? {
        guard let quantity, let price = valuationPrice else { return nil }
        let exposure = max(0, quantity - reducibleQuantity)
        return exposure * price * instrument.contractMultiplier / max(1, leverage)
    }

    var estimatedFee: Decimal? {
        guard let quantity, let price = valuationPrice else { return nil }
        switch commission {
        case .none: return nil
        case .fixedPerOrder(let amount): return amount
        case .percentage(let percent): return quantity * price * instrument.contractMultiplier * percent / 100
        case .perContract(let amount): return quantity * amount
        }
    }

    var exceedsFunds: Bool {
        guard let required = requiredMargin, let availableFunds else { return false }
        return required > availableFunds
    }

    // MARK: Validation

    /// Fields that hold something wrong. A field left empty is `missingFields`, not an issue, so a
    /// fresh ticket does not open covered in red.
    var issues: [Field: String] {
        var result: [Field: String] = [:]
        if !quantityText.trimmed.isEmpty {
            if let quantity {
                if quantity <= 0 {
                    result[.quantity] = "Must be greater than zero."
                } else if quantity < instrument.minimumQuantity {
                    result[.quantity] = "Minimum is \(formatQuantity(instrument.minimumQuantity))."
                } else if !Self.isAligned(quantity, to: instrument.quantityIncrement) {
                    result[.quantity] = "Must be a multiple of \(formatQuantity(instrument.quantityIncrement))."
                }
            } else {
                result[.quantity] = "Enter a number."
            }
        }
        if usesLimitPrice { result[.limitPrice] = priceIssue(limitPriceText, limitPrice) }
        if usesStopPrice { result[.stopPrice] = priceIssue(stopPriceText, stopPrice) }
        result[.takeProfit] = protectionIssue(takeProfitText, takeProfit, isTakeProfit: true)
        result[.stopLoss] = protectionIssue(stopLossText, stopLoss, isTakeProfit: false)
        return result.compactMapValues { $0 }
    }

    /// Required fields still empty.
    var missingFields: Set<Field> {
        var result: Set<Field> = []
        if quantityText.trimmed.isEmpty { result.insert(.quantity) }
        if usesLimitPrice, limitPriceText.trimmed.isEmpty { result.insert(.limitPrice) }
        if usesStopPrice, stopPriceText.trimmed.isEmpty { result.insert(.stopPrice) }
        return result
    }

    var canSubmit: Bool { quantity != nil && issues.isEmpty && missingFields.isEmpty && !exceedsFunds }

    // MARK: Actions

    /// The quantity that would use `fraction` (0...1) of the available funds at the valuation price,
    /// rounded down to the instrument's increment. Nil when funds, price or minimum size rule it out.
    func quantity(forFundsFraction fraction: Decimal) -> Decimal? {
        guard let availableFunds, availableFunds > 0, fraction > 0,
            let price = valuationPrice ?? marketPrice, price > 0
        else { return nil }
        let raw = availableFunds * min(1, fraction) * max(1, leverage) / (price * instrument.contractMultiplier)
        let rounded = Self.roundedDown(raw, toMultipleOf: instrument.quantityIncrement)
        return rounded >= instrument.minimumQuantity ? rounded : nil
    }

    func request(accountID: UUID) -> PaperOrderRequest? {
        guard let quantity else { return nil }
        return PaperOrderRequest(
            accountID: accountID, instrument: instrument, side: side, type: type, quantity: quantity,
            limitPrice: usesLimitPrice ? limitPrice : nil,
            stopPrice: usesStopPrice ? stopPrice : nil,
            timeInForce: timeInForce, takeProfit: takeProfit, stopLoss: stopLoss)
    }

    // MARK: Helpers

    private func priceIssue(_ text: String, _ value: Decimal?) -> String? {
        guard !text.trimmed.isEmpty else { return nil }
        guard let value else { return "Enter a number." }
        if value <= 0 { return "Must be greater than zero." }
        if !Self.isAligned(value, to: instrument.tickSize) {
            return "Must be a multiple of \(PaperTradingFormatter.price(instrument.tickSize, instrument: instrument))."
        }
        return nil
    }

    /// A take profit sits beyond the entry in the trade's favour; a stop loss sits behind it.
    private func protectionIssue(_ text: String, _ value: Decimal?, isTakeProfit: Bool) -> String? {
        if let issue = priceIssue(text, value) { return issue }
        guard let value, let entry = valuationPrice else { return nil }
        let above = (side == .buy) == isTakeProfit
        if above, value <= entry { return "Must be above \(formatPrice(entry))." }
        if !above, value >= entry { return "Must be below \(formatPrice(entry))." }
        return nil
    }

    private func formatPrice(_ value: Decimal) -> String { PaperTradingFormatter.price(value, instrument: instrument) }
    private func formatQuantity(_ value: Decimal) -> String {
        PaperTradingFormatter.quantity(value, instrument: instrument)
    }

    /// Same test the engine applies: the value divided by the increment must be a whole number.
    static func isAligned(_ value: Decimal, to increment: Decimal) -> Bool {
        guard increment > 0 else { return true }
        let quotient = value / increment
        return Decimal.rounded(quotient, scale: 8) == Decimal.rounded(quotient, scale: 0)
    }

    static func roundedDown(_ value: Decimal, toMultipleOf increment: Decimal) -> Decimal {
        guard increment > 0 else { return value }
        var quotient = value / increment
        var result = Decimal()
        NSDecimalRound(&result, &quotient, 0, .down)
        return result * increment
    }
}

extension String {
    fileprivate var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}
