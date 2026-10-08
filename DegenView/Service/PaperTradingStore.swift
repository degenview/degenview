import Foundation

@MainActor
final class PaperTradingStore: ObservableObject {
    static let shared = PaperTradingStore()

    @Published private(set) var snapshot: PaperTradingSnapshot
    @Published private(set) var metrics: PaperAccountMetrics?
    @Published var lastError: String?
    @Published var isConnected = false

    let engine: PaperTradingEngine
    let execution: PaperTradingExecutionService

    private struct PendingQuote {
        let instrument: PaperInstrument
        let bid: Decimal?
        let ask: Decimal?
        let last: Decimal?
        let timestamp: Date
    }
    private var pendingQuotes: [String: PendingQuote] = [:]
    private var isQuoteFlushScheduled = false
    private let quoteFlushInterval: TimeInterval

    init(database: AppDatabase = .shared, quoteFlushInterval: TimeInterval = 0.5) {
        self.quoteFlushInterval = quoteFlushInterval
        var initial = PaperTradingSnapshot.empty
        var canPersist = true
        do {
            initial = try database.paperTrading()
        } catch {
            // Never let an empty in-memory account overwrite one that failed to decode.
            canPersist = false
            #if DEBUG
                print("[PaperTradingStore] Paper trading unreadable; disabling writes: \(error.localizedDescription)")
            #endif
        }
        // A quote from an earlier run is hours old at best; showing it would put last session's price
        // in the order ticket until the first fresh one arrives.
        initial.quotes = [:]
        snapshot = initial
        let engine = PaperTradingEngine(
            snapshot: initial,
            persist: canPersist ? { @Sendable value in try database.savePaperTrading(value) } : nil)
        self.engine = engine
        execution = PaperTradingExecutionService(engine: engine)
    }

    var selectedAccount: PaperAccount? {
        guard let id = snapshot.selectedAccountID else { return nil }
        return snapshot.accounts.first { $0.id == id }
    }
    var positions: [PaperPosition] { scoped(snapshot.positions) }
    var workingOrders: [PaperOrder] { scoped(snapshot.orders).filter { $0.status.isWorking } }
    var orderHistory: [PaperOrderEvent] { scoped(snapshot.orderEvents).reversed() }
    var closedTrades: [PaperClosedTrade] { scoped(snapshot.closedTrades).reversed() }
    var journal: [PaperJournalEntry] { scoped(snapshot.journal).reversed() }

    func connect() async {
        do {
            if snapshot.accounts.isEmpty {
                _ = try await engine.createAccount(name: "Paper Trading", initialBalance: 100_000)
            }
            isConnected = true
            await refresh()
        } catch { report(error) }
    }

    func createAccount(
        name: String, currency: PaperCurrency, balance: Decimal,
        settings: PaperAccountSettings
    ) async {
        do {
            _ = try await engine.createAccount(
                name: name, currency: currency, initialBalance: balance, settings: settings)
            await refresh()
        } catch { report(error) }
    }

    func select(_ id: UUID) async {
        do {
            try await engine.selectAccount(id)
            await refresh()
        } catch { report(error) }
    }

    func submit(_ request: PaperOrderRequest) async -> Bool {
        do {
            _ = try await execution.submit(request)
            await refresh()
            return true
        } catch {
            report(error)
            await refresh()
            return false
        }
    }

    func cancel(_ id: UUID) async {
        do {
            try await execution.cancel(id)
            await refresh()
        } catch { report(error) }
    }

    func modify(_ id: UUID, changes: PaperOrderChanges) async {
        do {
            try await execution.modify(id, changes: changes)
            await refresh()
        } catch { report(error) }
    }

    func process(
        instrument: PaperInstrument, bid: Decimal? = nil, ask: Decimal? = nil,
        last: Decimal?, timestamp: Date, marketOpen: Bool = true
    ) async {
        do {
            try await engine.process(
                .init(
                    instrumentKey: instrument.key, bid: bid, ask: ask,
                    last: last, timestamp: timestamp, isMarketOpen: marketOpen))
            await refresh()
        } catch { report(error) }
    }

    /// Hands the engine a market's latest quote, at most once every `quoteFlushInterval` per
    /// instrument. Charts call this on every tick; the newest quote of each burst is the one used.
    /// Use `process` for a quote that must land now.
    func stream(instrument: PaperInstrument, bid: Decimal?, ask: Decimal?, last: Decimal?) {
        pendingQuotes[instrument.key] = PendingQuote(
            instrument: instrument, bid: bid, ask: ask, last: last, timestamp: Date())
        guard !isQuoteFlushScheduled else { return }
        isQuoteFlushScheduled = true
        let interval = quoteFlushInterval
        Task {
            try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
            await flushQuotes()
        }
    }

    /// Sends every pending streamed quote to the engine now.
    func flushQuotes() async {
        isQuoteFlushScheduled = false
        let batch = Array(pendingQuotes.values)
        pendingQuotes = [:]
        for quote in batch {
            await process(
                instrument: quote.instrument, bid: quote.bid, ask: quote.ask, last: quote.last,
                timestamp: quote.timestamp)
        }
    }

    func reset(currency: PaperCurrency, balance: Decimal, settings: PaperAccountSettings) async {
        guard let id = snapshot.selectedAccountID else { return }
        do {
            try await engine.resetAccount(id, currency: currency, initialBalance: balance, settings: settings)
            await refresh()
        } catch { report(error) }
    }

    func close(_ position: PaperPosition, fraction: Decimal = 1) async {
        let quantity = position.quantity * min(1, max(0, fraction))
        guard quantity > 0 else { return }
        _ = await submit(
            .init(
                accountID: position.accountID, instrument: position.instrument,
                side: position.signedQuantity > 0 ? .sell : .buy, type: .market, quantity: quantity))
    }

    func reverse(_ position: PaperPosition) async {
        _ = await submit(
            .init(
                accountID: position.accountID, instrument: position.instrument,
                side: position.signedQuantity > 0 ? .sell : .buy, type: .market, quantity: position.quantity * 2))
    }

    func refresh() async {
        snapshot = await engine.snapshot()
        if let id = snapshot.selectedAccountID { metrics = await engine.metrics(accountID: id) } else { metrics = nil }
    }

    func clearError() { lastError = nil }

    // MARK: Formatting

    var accountCurrency: PaperCurrency { selectedAccount?.baseCurrency ?? .USD }

    func money(_ value: Decimal) -> String {
        PaperTradingFormatter.money(value, currency: accountCurrency)
    }

    func signedMoney(_ value: Decimal) -> String {
        PaperTradingFormatter.signedMoney(value, currency: accountCurrency)
    }

    // MARK: Marks

    /// The price a position would close at right now: the bid for a long, the ask for a short,
    /// falling back to the last trade. Nil until the instrument has a quote.
    func mark(for position: PaperPosition) -> Decimal? {
        guard let quote = snapshot.quotes[position.instrument.key] else { return nil }
        return position.signedQuantity >= 0 ? (quote.bid ?? quote.last) : (quote.ask ?? quote.last)
    }

    /// Open profit or loss, the same figure the engine reports in its metrics (zero without a quote).
    func unrealizedPnL(for position: PaperPosition) -> Decimal {
        guard let mark = mark(for: position) else { return 0 }
        let difference =
            position.signedQuantity >= 0
            ? mark - position.averageEntryPrice : position.averageEntryPrice - mark
        return difference * position.quantity * position.instrument.pointValue
    }

    /// Open profit or loss as a ratio of the position's entry value (`0.05` is +5%).
    func returnRatio(for position: PaperPosition) -> Decimal? {
        let cost = position.averageEntryPrice * position.quantity * position.instrument.pointValue
        guard cost > 0, mark(for: position) != nil else { return nil }
        return unrealizedPnL(for: position) / cost
    }

    private func scoped<T>(_ values: [T]) -> [T] {
        guard let id = snapshot.selectedAccountID else { return [] }
        return values.filter {
            switch $0 {
            case let value as PaperPosition: value.accountID == id
            case let value as PaperOrder: value.accountID == id
            case let value as PaperOrderEvent: value.accountID == id
            case let value as PaperClosedTrade: value.accountID == id
            case let value as PaperJournalEntry: value.accountID == id
            default: false
            }
        }
    }

    private func report(_ error: Error) { lastError = error.localizedDescription }
}
