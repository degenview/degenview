import Foundation

protocol TradingExecutionService: Sendable {
    var environment: TradingExecutionEnvironment { get }
    func submit(_ order: PaperOrderRequest) async throws -> PaperOrderID
    func cancel(_ orderID: PaperOrderID) async throws
    func modify(_ orderID: PaperOrderID, changes: PaperOrderChanges) async throws
}

enum TradingExecutionEnvironment: String, Sendable { case paper, live }

/// The only execution adapter accepted by paper-trading UI. It owns no credentials,
/// URLs, or exchange clients and can route exclusively to the local actor.
final class PaperTradingExecutionService: TradingExecutionService, @unchecked Sendable {
    let environment: TradingExecutionEnvironment = .paper
    private let engine: PaperTradingEngine

    init(engine: PaperTradingEngine) { self.engine = engine }
    func submit(_ order: PaperOrderRequest) async throws -> PaperOrderID { try await engine.submit(order) }
    func cancel(_ orderID: PaperOrderID) async throws { try await engine.cancel(orderID) }
    func modify(_ orderID: PaperOrderID, changes: PaperOrderChanges) async throws {
        try await engine.modify(orderID, changes: changes)
    }
}

actor PaperTradingEngine {
    typealias Persistence = @Sendable (PaperTradingSnapshot) async throws -> Void

    private var state: PaperTradingSnapshot
    private let persist: Persistence?
    private let now: @Sendable () -> Date
    private let quoteMaximumAge: TimeInterval

    init(
        snapshot: PaperTradingSnapshot = .empty, quoteMaximumAge: TimeInterval = 30,
        now: @escaping @Sendable () -> Date = { Date() }, persist: Persistence? = nil
    ) {
        self.state = snapshot
        self.quoteMaximumAge = quoteMaximumAge
        self.now = now
        self.persist = persist
    }

    func snapshot() -> PaperTradingSnapshot { state }

    @discardableResult
    func createAccount(
        name: String, currency: PaperCurrency = .USD, initialBalance: Decimal = 100_000,
        settings: PaperAccountSettings = .init()
    ) async throws -> UUID {
        guard initialBalance > 0 else {
            throw PaperTradingError.invalidQuantity("Initial balance must be greater than zero.")
        }
        let account = PaperAccount(
            name: name.isEmpty ? "Paper Trading" : name, baseCurrency: currency,
            initialBalance: initialBalance, settings: settings, createdAt: now())
        state.accounts.append(account)
        state.selectedAccountID = account.id
        journal(account.id, "PAPER account created with \(initialBalance) \(currency.rawValue)")
        try await save()
        return account.id
    }

    func selectAccount(_ id: UUID) async throws {
        guard state.accounts.contains(where: { $0.id == id }) else { throw PaperTradingError.accountNotFound }
        state.selectedAccountID = id
        try await save()
    }

    func updateSettings(accountID: UUID, settings: PaperAccountSettings) async throws {
        guard let index = accountIndex(accountID) else { throw PaperTradingError.accountNotFound }
        state.accounts[index].settings = settings
        journal(accountID, "PAPER account settings changed; changes apply prospectively")
        try await save()
    }

    func resetAccount(
        _ id: UUID, currency: PaperCurrency, initialBalance: Decimal,
        settings: PaperAccountSettings
    ) async throws {
        guard let index = accountIndex(id) else { throw PaperTradingError.accountNotFound }
        guard initialBalance > 0 else {
            throw PaperTradingError.invalidQuantity("Initial balance must be greater than zero.")
        }
        let name = state.accounts[index].name
        state.accounts[index] = PaperAccount(
            id: id, name: name, baseCurrency: currency,
            initialBalance: initialBalance, settings: settings, createdAt: now())
        state.orders.removeAll { $0.accountID == id }
        state.fills.removeAll { $0.accountID == id }
        state.positions.removeAll { $0.accountID == id }
        state.orderEvents.removeAll { $0.accountID == id }
        state.closedTrades.removeAll { $0.accountID == id }
        state.journal.removeAll { $0.accountID == id }
        journal(id, "PAPER account reset to \(initialBalance) \(currency.rawValue)")
        try await save()
    }

    func deleteAccount(_ id: UUID) async throws {
        guard state.accounts.contains(where: { $0.id == id }) else { throw PaperTradingError.accountNotFound }
        state.accounts.removeAll { $0.id == id }
        state.orders.removeAll { $0.accountID == id }
        state.fills.removeAll { $0.accountID == id }
        state.positions.removeAll { $0.accountID == id }
        state.orderEvents.removeAll { $0.accountID == id }
        state.closedTrades.removeAll { $0.accountID == id }
        state.journal.removeAll { $0.accountID == id }
        if state.selectedAccountID == id { state.selectedAccountID = state.accounts.first?.id }
        try await save()
    }

    @discardableResult
    func submit(_ request: PaperOrderRequest) async throws -> PaperOrderID {
        let account = try account(request.accountID)
        try validate(request, account: account)
        let timestamp = now()
        var order = PaperOrder(
            id: UUID(), accountID: request.accountID, instrument: request.instrument, side: request.side,
            type: request.type, originalQuantity: request.quantity, filledQuantity: 0, averageFillPrice: nil,
            limitPrice: request.limitPrice, stopPrice: request.stopPrice, timeInForce: request.timeInForce,
            status: .pendingSubmission, role: .entry, parentOrderID: nil, ocoGroupID: nil,
            stopTriggered: false, reservedMargin: 0, rejectionReason: nil,
            createdAt: timestamp, updatedAt: timestamp
        )
        state.orders.append(order)
        event(
            order, .placed,
            "\(request.side.rawValue.uppercased()) \(request.quantity) \(request.instrument.symbol) \(request.type.rawValue) order submitted"
        )

        do {
            order.reservedMargin = try requiredMargin(for: order, account: account)
            let available = metrics(accountID: account.id).availableFunds
            guard order.reservedMargin <= available else {
                throw PaperTradingError.insufficientFunds(required: order.reservedMargin, available: available)
            }
            order.status = .working
            order.updatedAt = timestamp
            replace(order)
            event(order, .accepted, "PAPER order accepted")
            try evaluate(orderID: order.id, bracket: (request.takeProfit, request.stopLoss))
            try await save()
            return order.id
        } catch {
            order.status = .rejected
            order.rejectionReason = error.localizedDescription
            order.updatedAt = timestamp
            replace(order)
            event(order, .rejected, error.localizedDescription)
            try await save()
            throw error
        }
    }

    func cancel(_ orderID: UUID) async throws {
        guard var order = state.orders.first(where: { $0.id == orderID }) else { throw PaperTradingError.orderNotFound }
        guard order.status.isWorking else { throw PaperTradingError.invalidTransition }
        order.status = .pendingCancel
        order.updatedAt = now()
        replace(order)
        order.status = .canceled
        order.reservedMargin = 0
        replace(order)
        event(order, .canceled, "PAPER order canceled")
        try await save()
    }

    func modify(_ orderID: UUID, changes: PaperOrderChanges) async throws {
        guard var order = state.orders.first(where: { $0.id == orderID }) else { throw PaperTradingError.orderNotFound }
        guard order.status.isWorking else { throw PaperTradingError.invalidTransition }
        if let quantity = changes.quantity {
            guard quantity >= order.filledQuantity, quantity > 0 else {
                throw PaperTradingError.invalidQuantity("Quantity cannot be below the already-filled quantity.")
            }
            try validateStep(quantity, increment: order.instrument.quantityIncrement, field: "Quantity")
            order.originalQuantity = quantity
        }
        if let price = changes.limitPrice {
            try validatePrice(price, instrument: order.instrument)
            order.limitPrice = price
        }
        if let price = changes.stopPrice {
            try validatePrice(price, instrument: order.instrument)
            order.stopPrice = price
        }
        order.updatedAt = now()
        order.reservedMargin = try requiredMargin(for: order, account: try account(order.accountID))
        replace(order)
        event(order, .modified, "PAPER order modified")
        try evaluate(orderID: order.id, bracket: nil)
        try await save()
    }

    /// Serialized by this actor: an order cannot be evaluated twice concurrently.
    func process(_ quote: PaperQuote) async throws {
        if let previous = state.quotes[quote.instrumentKey], quote.timestamp < previous.timestamp { return }
        state.quotes[quote.instrumentKey] = quote
        let ids = state.orders.filter { $0.instrument.key == quote.instrumentKey && $0.status.isWorking }
            .sorted { $0.createdAt < $1.createdAt }.map(\.id)
        for id in ids { try evaluate(orderID: id, bracket: nil) }
        try await save()
    }

    func metrics(accountID: UUID) -> PaperAccountMetrics {
        guard let account = state.accounts.first(where: { $0.id == accountID }) else {
            return .init(
                balance: 0, equity: 0, realizedPnL: 0, unrealizedPnL: 0,
                positionMargin: 0, ordersMargin: 0, availableFunds: 0, marginBuffer: 0)
        }
        let positions = state.positions.filter { $0.accountID == accountID }
        let unrealized = positions.reduce(Decimal.zero) { $0 + unrealizedPnL($1) }
        let positionMargin = positions.reduce(Decimal.zero) { partial, position in
            let mark = liquidationPrice(position) ?? position.averageEntryPrice
            let leverage = max(1, account.settings.leverage.leverage(for: position.instrument.assetClass))
            return partial + position.quantity * mark * position.instrument.contractMultiplier / leverage
        }
        let ordersMargin = state.orders.filter { $0.accountID == accountID && $0.status.isWorking }
            .reduce(Decimal.zero) { $0 + $1.reservedMargin }
        let equity = account.cashBalance + unrealized
        let available = max(0, equity - positionMargin - ordersMargin)
        return .init(
            balance: account.cashBalance, equity: equity, realizedPnL: account.realizedPnL,
            unrealizedPnL: unrealized, positionMargin: positionMargin, ordersMargin: ordersMargin,
            availableFunds: available, marginBuffer: equity == 0 ? 0 : available / abs(equity))
    }

    func unrealizedPnL(_ position: PaperPosition) -> Decimal {
        guard let liquidation = liquidationPrice(position) else { return 0 }
        let difference =
            position.signedQuantity >= 0
            ? liquidation - position.averageEntryPrice
            : position.averageEntryPrice - liquidation
        return difference * position.quantity * position.instrument.pointValue
    }

    // MARK: Execution

    private func evaluate(orderID: UUID, bracket: (Decimal?, Decimal?)?) throws {
        guard var order = state.orders.first(where: { $0.id == orderID }), order.status.isWorking else { return }
        guard let quote = state.quotes[order.instrument.key] else {
            if order.type == .market { throw PaperTradingError.noMarketData }
            return
        }
        guard quote.isMarketOpen else { return }
        guard now().timeIntervalSince(quote.timestamp) <= quoteMaximumAge else {
            if order.type == .market { throw PaperTradingError.staleMarketData }
            return
        }
        guard let (executable, source) = quote.executablePrice(for: order.side) else { return }

        var fillPrice: Decimal?
        switch order.type {
        case .market: fillPrice = executable
        case .limit:
            if limitEligible(order, executable: executable) { fillPrice = betterPrice(order, executable: executable) }
        case .stop:
            if stopEligible(order, executable: executable) { fillPrice = executable }
        case .stopLimit:
            if !order.stopTriggered, stopEligible(order, executable: executable) {
                order.stopTriggered = true
                order.updatedAt = now()
                replace(order)
                event(order, .modified, "Stop triggered; PAPER stop-limit activated")
            }
            if order.stopTriggered, limitEligible(order, executable: executable) {
                fillPrice = betterPrice(order, executable: executable)
            }
        }
        guard var price = fillPrice else { return }
        let ticks = (try? account(order.accountID).settings.slippageTicks) ?? 0
        if ticks != 0 { price += (order.side == .buy ? 1 : -1) * ticks * order.instrument.tickSize }
        try fill(&order, quantity: order.remainingQuantity, price: price, source: source, bracket: bracket)
    }

    private func fill(
        _ order: inout PaperOrder, quantity: Decimal, price: Decimal,
        source: PaperFillPriceSource, bracket: (Decimal?, Decimal?)?
    ) throws {
        guard quantity > 0, order.status.isWorking else { return }
        let account = try account(order.accountID)
        let commission = commission(
            for: account.settings.commission, quantity: quantity,
            price: price, instrument: order.instrument)
        let fill = PaperFill(
            id: UUID(), orderID: order.id, accountID: order.accountID,
            instrument: order.instrument, side: order.side, quantity: quantity,
            price: price, commission: commission, priceSource: source, timestamp: now())
        let priorFilled = order.filledQuantity
        order.filledQuantity += quantity
        order.averageFillPrice = ((order.averageFillPrice ?? 0) * priorFilled + price * quantity) / order.filledQuantity
        order.status = order.remainingQuantity == 0 ? .filled : .partiallyFilled
        order.reservedMargin = 0
        order.updatedAt = now()
        replace(order)
        state.fills.append(fill)
        let realized = applyPosition(fill)
        guard let accountIndex = accountIndex(order.accountID) else { throw PaperTradingError.accountNotFound }
        state.accounts[accountIndex].cashBalance += realized - commission
        state.accounts[accountIndex].realizedPnL += realized - commission
        event(
            order, order.status == .filled ? .filled : .partiallyFilled,
            "Filled \(quantity) \(order.instrument.symbol) @ \(price) [\(source.rawValue)]; commission \(commission)")
        if realized != 0 { journal(order.accountID, "Realized gross P&L \(realized) before commission") }
        if order.status == .filled {
            cancelOCOCounterpart(of: order)
            if order.role == .entry, let bracket {
                createBracketChildren(parent: order, takeProfit: bracket.0, stopLoss: bracket.1)
            }
        }
    }

    @discardableResult
    private func applyPosition(_ fill: PaperFill) -> Decimal {
        let delta = fill.side == .buy ? fill.quantity : -fill.quantity
        if let index = state.positions.firstIndex(where: {
            $0.accountID == fill.accountID && $0.instrument.key == fill.instrument.key
        }) {
            var position = state.positions[index]
            let oldSigned = position.signedQuantity
            let sameDirection = (oldSigned >= 0 && delta > 0) || (oldSigned <= 0 && delta < 0)
            if sameDirection {
                let total = abs(oldSigned) + abs(delta)
                position.averageEntryPrice =
                    (position.averageEntryPrice * abs(oldSigned) + fill.price * abs(delta)) / total
                position.signedQuantity += delta
                position.commissions += fill.commission
                position.updatedAt = fill.timestamp
                state.positions[index] = position
                return 0
            }
            let closedQuantity = min(abs(oldSigned), abs(delta))
            let gross =
                (oldSigned > 0 ? fill.price - position.averageEntryPrice : position.averageEntryPrice - fill.price)
                * closedQuantity * position.instrument.pointValue
            let allocatedEntryCommission =
                position.quantity == 0 ? 0 : position.commissions * closedQuantity / position.quantity
            let closingCommission = fill.quantity == 0 ? 0 : fill.commission * closedQuantity / fill.quantity
            let openingCommission = fill.commission - closingCommission
            let tradeCommission = allocatedEntryCommission + closingCommission
            state.closedTrades.append(
                .init(
                    id: UUID(), accountID: fill.accountID, instrument: fill.instrument,
                    side: oldSigned > 0 ? .long : .short, entryTimestamp: position.openedAt,
                    exitTimestamp: fill.timestamp,
                    entryPrice: position.averageEntryPrice, exitPrice: fill.price, quantity: closedQuantity,
                    grossPnL: gross, commission: tradeCommission))
            position.realizedGrossPnL += gross
            position.commissions -= allocatedEntryCommission
            position.signedQuantity += delta
            position.updatedAt = fill.timestamp
            if position.signedQuantity == 0 {
                state.positions.remove(at: index)
            } else if (oldSigned > 0) != (position.signedQuantity > 0) {
                position.averageEntryPrice = fill.price
                position.openedAt = fill.timestamp
                position.commissions = openingCommission
                state.positions[index] = position
            } else {
                state.positions[index] = position
            }
            resizeProtectiveOrders(
                accountID: fill.accountID, instrumentKey: fill.instrument.key,
                remaining: abs(position.signedQuantity))
            return gross
        }
        state.positions.append(
            .init(
                accountID: fill.accountID, instrument: fill.instrument,
                signedQuantity: delta, averageEntryPrice: fill.price, realizedGrossPnL: 0,
                commissions: fill.commission, openedAt: fill.timestamp, updatedAt: fill.timestamp))
        return 0
    }

    private func createBracketChildren(parent: PaperOrder, takeProfit: Decimal?, stopLoss: Decimal?) {
        guard takeProfit != nil || stopLoss != nil else { return }
        let group = UUID()
        let exitSide: PaperOrderSide = parent.side == .buy ? .sell : .buy
        func append(type: PaperOrderType, price: Decimal, role: PaperOrderRole) {
            var child = PaperOrder(
                id: UUID(), accountID: parent.accountID, instrument: parent.instrument,
                side: exitSide, type: type, originalQuantity: parent.filledQuantity, filledQuantity: 0,
                averageFillPrice: nil, limitPrice: type == .limit ? price : nil,
                stopPrice: type == .stop ? price : nil, timeInForce: .goodTilCanceled,
                status: .working, role: role, parentOrderID: parent.id, ocoGroupID: group,
                stopTriggered: false, reservedMargin: 0, rejectionReason: nil,
                createdAt: now(), updatedAt: now())
            child.reservedMargin = 0  // reduce-only protection never reserves new exposure
            state.orders.append(child)
            event(child, .accepted, "PAPER \(role.rawValue) protection activated")
        }
        if let takeProfit { append(type: .limit, price: takeProfit, role: .takeProfit) }
        if let stopLoss { append(type: .stop, price: stopLoss, role: .stopLoss) }
    }

    private func cancelOCOCounterpart(of order: PaperOrder) {
        guard let group = order.ocoGroupID else { return }
        for index in state.orders.indices
        where state.orders[index].ocoGroupID == group && state.orders[index].id != order.id
            && state.orders[index].status.isWorking
        {
            state.orders[index].status = .canceled
            state.orders[index].reservedMargin = 0
            state.orders[index].updatedAt = now()
            event(state.orders[index], .canceled, "PAPER OCO counterpart canceled")
        }
    }

    private func resizeProtectiveOrders(accountID: UUID, instrumentKey: String, remaining: Decimal) {
        for index in state.orders.indices
        where state.orders[index].accountID == accountID && state.orders[index].instrument.key == instrumentKey
            && state.orders[index].role != .entry && state.orders[index].status.isWorking
        {
            if remaining == 0 {
                state.orders[index].status = .canceled
            } else {
                state.orders[index].originalQuantity = state.orders[index].filledQuantity + remaining
            }
            state.orders[index].updatedAt = now()
        }
    }

    // MARK: Validation / calculations

    private func validate(_ request: PaperOrderRequest, account: PaperAccount) throws {
        guard request.quantity >= request.instrument.minimumQuantity else {
            throw PaperTradingError.invalidQuantity("Quantity must be at least \(request.instrument.minimumQuantity).")
        }
        try validateStep(request.quantity, increment: request.instrument.quantityIncrement, field: "Quantity")
        if request.instrument.quoteCurrency != account.baseCurrency,
            !([PaperCurrency.USDT, .USDC].contains(request.instrument.quoteCurrency) && account.baseCurrency == .USD)
        {
            throw PaperTradingError.unsupportedCurrencyConversion(
                request.instrument.quoteCurrency, account.baseCurrency)
        }
        switch request.type {
        case .market: break
        case .limit:
            guard let price = request.limitPrice else {
                throw PaperTradingError.invalidPrice("A limit price is required.")
            }
            try validatePrice(price, instrument: request.instrument)
        case .stop:
            guard let price = request.stopPrice else {
                throw PaperTradingError.invalidPrice("A stop price is required.")
            }
            try validatePrice(price, instrument: request.instrument)
        case .stopLimit:
            guard let limit = request.limitPrice, let stop = request.stopPrice else {
                throw PaperTradingError.invalidPrice("Stop and limit prices are required.")
            }
            try validatePrice(limit, instrument: request.instrument)
            try validatePrice(stop, instrument: request.instrument)
        }
        if let price = request.takeProfit { try validatePrice(price, instrument: request.instrument) }
        if let price = request.stopLoss { try validatePrice(price, instrument: request.instrument) }
    }

    private func validatePrice(_ value: Decimal, instrument: PaperInstrument) throws {
        guard value > 0 else { throw PaperTradingError.invalidPrice("Price must be greater than zero.") }
        try validateStep(value, increment: instrument.tickSize, field: "Price")
    }

    private func validateStep(_ value: Decimal, increment: Decimal, field: String) throws {
        guard increment > 0 else { return }
        let quotient = value / increment
        guard Decimal.rounded(quotient, scale: 8) == Decimal.rounded(quotient, scale: 0) else {
            throw field == "Price"
                ? PaperTradingError.invalidPrice("Price must align to tick size \(increment).")
                : PaperTradingError.invalidQuantity("Quantity must align to increment \(increment).")
        }
    }

    private func requiredMargin(for order: PaperOrder, account: PaperAccount) throws -> Decimal {
        if order.role != .entry { return 0 }
        let price: Decimal
        if order.type == .limit || order.type == .stopLimit, let limit = order.limitPrice {
            price = limit
        } else if order.type == .stop, let stop = order.stopPrice {
            price = stop
        } else if let quote = state.quotes[order.instrument.key]?.executablePrice(for: order.side)?.0 {
            price = quote
        } else {
            throw PaperTradingError.noMarketData
        }
        let signedPosition =
            state.positions.first {
                $0.accountID == order.accountID && $0.instrument.key == order.instrument.key
            }?.signedQuantity ?? 0
        let reducesExisting = (signedPosition > 0 && order.side == .sell) || (signedPosition < 0 && order.side == .buy)
        let reducibleQuantity = reducesExisting ? abs(signedPosition) : 0
        let exposureIncreasingQuantity = max(0, order.remainingQuantity - reducibleQuantity)
        let leverage = max(1, account.settings.leverage.leverage(for: order.instrument.assetClass))
        return exposureIncreasingQuantity * price * order.instrument.contractMultiplier / leverage
    }

    private func commission(
        for config: PaperCommissionConfiguration, quantity: Decimal, price: Decimal,
        instrument: PaperInstrument
    ) -> Decimal {
        switch config {
        case .none: 0
        case .fixedPerOrder(let amount): amount
        case .percentage(let percent): quantity * price * instrument.contractMultiplier * percent / 100
        case .perContract(let amount): quantity * amount
        }
    }

    private func liquidationPrice(_ position: PaperPosition) -> Decimal? {
        guard let quote = state.quotes[position.instrument.key] else { return nil }
        return position.signedQuantity >= 0 ? (quote.bid ?? quote.last) : (quote.ask ?? quote.last)
    }

    private func stopEligible(_ order: PaperOrder, executable: Decimal) -> Bool {
        guard let stop = order.stopPrice else { return false }
        return order.side == .buy ? executable >= stop : executable <= stop
    }
    private func limitEligible(_ order: PaperOrder, executable: Decimal) -> Bool {
        guard let limit = order.limitPrice else { return false }
        return order.side == .buy ? executable <= limit : executable >= limit
    }
    private func betterPrice(_ order: PaperOrder, executable: Decimal) -> Decimal {
        guard let limit = order.limitPrice else { return executable }
        return order.side == .buy ? min(executable, limit) : max(executable, limit)
    }

    private func account(_ id: UUID) throws -> PaperAccount {
        guard let account = state.accounts.first(where: { $0.id == id }) else {
            throw PaperTradingError.accountNotFound
        }
        return account
    }
    private func accountIndex(_ id: UUID) -> Int? { state.accounts.firstIndex { $0.id == id } }
    private func replace(_ order: PaperOrder) {
        if let index = state.orders.firstIndex(where: { $0.id == order.id }) { state.orders[index] = order }
    }
    private func event(_ order: PaperOrder, _ kind: PaperOrderEventKind, _ message: String) {
        state.orderEvents.append(
            .init(
                id: UUID(), orderID: order.id, accountID: order.accountID,
                kind: kind, timestamp: now(), message: message, order: order))
        journal(order.accountID, message)
    }
    private func journal(_ accountID: UUID, _ message: String) {
        state.journal.append(.init(id: UUID(), accountID: accountID, timestamp: now(), message: message))
    }
    private func save() async throws { try await persist?(state) }
}
