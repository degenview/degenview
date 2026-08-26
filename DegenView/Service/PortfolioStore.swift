import Foundation
import SwiftUI

@MainActor
final class PortfolioStore: ObservableObject {
    static let shared = PortfolioStore()

    @Published private(set) var snapshot: PortfolioLedgerSnapshot
    @Published private(set) var quotes: [String: PortfolioQuote]
    @Published private(set) var isRefreshing = false
    @Published private(set) var isLoadingInitialValues: Bool
    @Published var lastError: String?
    @AppStorage("portfolioPrivacyMode") var privacyMode = false

    let ledger: PortfolioLedger

    private struct DerivedState {
        let transactions: [PortfolioTransaction]
        let holdings: [PortfolioHolding]
        let history: [PortfolioSnapshot]
        let totalValue: Decimal
        let totalRealizedPnL: Decimal
        let totalUnrealizedPnL: Decimal
        let unpricedAssetCount: Int
    }
    private var derivedStates: [Set<UUID>: DerivedState] = [:]
    private var historyTasks: [UUID: Task<Void, Never>] = [:]
    private var quoteTasks: [String: Task<PortfolioQuote?, Never>] = [:]
    private let quoteFreshness: TimeInterval = 5 * 60
    private let quoteStore = JSONStore<[String: PortfolioQuote]>(filename: "portfolio_quotes.json")

    private init() {
        let initial = JSONStore<PortfolioLedgerSnapshot>(filename: "portfolios.json").load() ?? .empty
        let loadedQuotes = JSONStore<[String: PortfolioQuote]>(filename: "portfolio_quotes.json").load() ?? [:]
        let referencedKeys = Set(PortfolioAccountingEngine.uniqueAssets(in: initial.transactions).map(\.key))
        let initialQuotes = loadedQuotes.filter { referencedKeys.contains($0.key) }
        snapshot = initial
        quotes = initialQuotes
        isLoadingInitialValues = Self.needsInitialQuoteLoad(snapshot: initial, quotes: initialQuotes)
        ledger = PortfolioLedger(
            snapshot: initial,
            persist: { value in
                JSONStore<PortfolioLedgerSnapshot>(filename: "portfolios.json").save(value)
            })
        if initialQuotes != loadedQuotes {
            JSONStore<[String: PortfolioQuote]>(filename: "portfolio_quotes.json").save(initialQuotes)
        }
    }

    var selectedPortfolio: Portfolio? {
        snapshot.selectedPortfolioID.flatMap { id in snapshot.portfolios.first { $0.id == id } }
    }
    var selection: PortfolioSelection { snapshot.selectedPortfolioID.map(PortfolioSelection.portfolio) ?? .all }
    var activePortfolios: [Portfolio] { snapshot.portfolios.filter { !$0.isArchived } }
    var selectedPortfolioIDs: Set<UUID> {
        if let id = snapshot.selectedPortfolioID { return [id] }
        return Set(activePortfolios.map(\.id))
    }
    var transactions: [PortfolioTransaction] { derivedState(for: selectedPortfolioIDs).transactions }
    var holdings: [PortfolioHolding] { derivedState(for: selectedPortfolioIDs).holdings }
    var totalValue: Decimal { derivedState(for: selectedPortfolioIDs).totalValue }
    var totalRealizedPnL: Decimal { derivedState(for: selectedPortfolioIDs).totalRealizedPnL }
    var totalUnrealizedPnL: Decimal { derivedState(for: selectedPortfolioIDs).totalUnrealizedPnL }
    var totalPnL: Decimal { totalRealizedPnL + totalUnrealizedPnL }
    var unpricedAssetCount: Int { derivedState(for: selectedPortfolioIDs).unpricedAssetCount }
    var marketValueCaption: String {
        if unpricedAssetCount > 0 {
            return "+ \(unpricedAssetCount) unpriced asset\(unpricedAssetCount == 1 ? "" : "s")"
        }
        let now = Date()
        let assets = Self.quoteEligibleAssets(in: snapshot, portfolioIDs: selectedPortfolioIDs)
        return assets.contains { asset in
            quotes[asset.key].map { now.timeIntervalSince($0.timestamp) >= quoteFreshness } ?? false
        } ? "Cached market value" : "Live market value"
    }

    func portfolio(namedBy id: UUID?) -> Portfolio? {
        guard let id else { return nil }
        return activePortfolios.first { $0.id == id }
    }

    func holdings(for portfolioID: UUID?) -> [PortfolioHolding] {
        let ids = portfolioID.map { Set([$0]) } ?? Set(activePortfolios.map(\.id))
        return derivedState(for: ids).holdings
    }

    func history(for portfolioID: UUID?) -> [PortfolioSnapshot] {
        let ids = portfolioID.map { Set([$0]) } ?? Set(activePortfolios.map(\.id))
        return derivedState(for: ids).history
    }

    func refresh() async {
        let latest = await ledger.snapshot()
        guard latest != snapshot else { return }
        snapshot = latest
        derivedStates.removeAll(keepingCapacity: true)
        pruneQuotes()
    }
    func select(_ selection: PortfolioSelection) { perform { try await self.ledger.select(selection.id) } }
    func create(name: String, currency: PortfolioCurrency) {
        perform { _ = try await self.ledger.createPortfolio(name: name, currency: currency) }
    }
    func update(_ portfolio: Portfolio) { perform { try await self.ledger.updatePortfolio(portfolio) } }
    func duplicate(_ id: UUID) { perform { _ = try await self.ledger.duplicatePortfolio(id) } }
    func delete(_ id: UUID) { perform { try await self.ledger.deletePortfolio(id) } }
    func reorder(from: IndexSet, to: Int) { perform { try await self.ledger.reorder(from: from, to: to) } }
    func add(_ transaction: PortfolioTransaction) async -> Bool {
        await result { try await self.ledger.add(transaction) }
    }
    func update(_ transaction: PortfolioTransaction) async -> Bool {
        await result { try await self.ledger.update(transaction) }
    }
    func deleteTransaction(_ id: UUID) { perform { try await self.ledger.deleteTransaction(id) } }
    func remapAsset(
        from sourceKey: String, to destination: PortfolioAsset,
        portfolioIDs: Set<UUID>
    ) async throws {
        do {
            try await ledger.remapAsset(from: sourceKey, to: destination, portfolioIDs: portfolioIDs)
            await refresh()
        } catch {
            lastError = error.localizedDescription
            throw error
        }
    }
    func importTransactions(_ values: [PortfolioTransaction]) async -> Bool {
        await result { try await self.ledger.importTransactions(values) }
    }

    func refreshQuotes() async {
        defer { isLoadingInitialValues = false }
        await refreshQuotes(for: selectedPortfolioIDs)
    }

    func refreshQuotes(forPortfolioID id: UUID?) async {
        let ids = id.map { Set([$0]) } ?? Set(activePortfolios.map(\.id))
        await refreshQuotes(for: ids)
    }

    private func refreshQuotes(for portfolioIDs: Set<UUID>) async {
        let relevantTransactions = snapshot.transactions.filter { portfolioIDs.contains($0.portfolioID) }
        let assets = PortfolioAccountingEngine.uniqueAssets(in: relevantTransactions)
        guard !assets.isEmpty else {
            pruneQuotes()
            return
        }
        isRefreshing = true
        defer { isRefreshing = false }
        let now = Date()
        var updates: [String: PortfolioQuote] = [:]
        await withTaskGroup(of: (String, PortfolioQuote?).self) { group in
            for asset in assets {
                if let quote = quotes[asset.key], now.timeIntervalSince(quote.timestamp) < quoteFreshness { continue }
                let task: Task<PortfolioQuote?, Never>
                if let existing = quoteTasks[asset.key] {
                    task = existing
                } else {
                    task = Task {
                        guard asset.quoteCurrency == .USD else { return nil }
                        let service = DataSourceFactory.shared.service(for: asset.source)
                        guard
                            let data = try? await service.fetchKlines(
                                symbol: asset.key.components(separatedBy: ":").dropFirst().joined(separator: ":"),
                                interval: "1h", limit: 25),
                            let latest = data.last
                        else { return nil }
                        let prior = data.last(where: { latest.openTime.timeIntervalSince($0.openTime) >= 23 * 3600 })
                        return PortfolioQuote(
                            price: Decimal(latest.closePrice),
                            previousDayPrice: prior.map { Decimal($0.closePrice) }, timestamp: Date())
                    }
                    quoteTasks[asset.key] = task
                }
                group.addTask {
                    (asset.key, await task.value)
                }
            }
            for await (key, quote) in group { if let quote { updates[key] = quote } }
        }
        for asset in assets { quoteTasks[asset.key] = nil }
        if !updates.isEmpty {
            var merged = quotes
            merged.merge(updates) { _, new in new }
            if merged != quotes {
                quotes = merged
                derivedStates.removeAll(keepingCapacity: true)
                quoteStore.save(merged)
            }
        }
        pruneQuotes()
    }

    /// Rebuilds only the invalidated suffix. Each daily point uses holdings that existed then
    /// and a nearest prior real close; missing observations produce an incomplete snapshot.
    func rebuildHistory() async {
        let portfolios = selectedPortfolio.map { [$0] } ?? activePortfolios
        for portfolio in portfolios { await rebuildHistory(for: portfolio) }
    }

    func rebuildHistory(forPortfolioID id: UUID?) async {
        let portfolios =
            id.flatMap { target in activePortfolios.first { $0.id == target } }.map { [$0] } ?? activePortfolios
        for portfolio in portfolios { await rebuildHistory(for: portfolio) }
    }

    private func rebuildHistory(for portfolio: Portfolio) async {
        if let task = historyTasks[portfolio.id] {
            await task.value
            return
        }
        guard let start = Self.historyRebuildStart(for: portfolio.id, in: snapshot, today: Date()) else { return }
        let task = Task { [weak self] in
            guard let self else { return }
            await self.performHistoryRebuild(for: portfolio, start: start)
        }
        historyTasks[portfolio.id] = task
        await task.value
        historyTasks[portfolio.id] = nil
    }

    private func performHistoryRebuild(for portfolio: Portfolio, start: Date) async {
        let calendar = Calendar(identifier: .gregorian)
        let end = Date()
        let assets = PortfolioAccountingEngine.uniqueAssets(
            in: snapshot.transactions.filter { $0.portfolioID == portfolio.id }
        )
        var histories: [String: [KlineData]] = [:]
        if portfolio.baseCurrency == .USD {
            await withTaskGroup(of: (String, [KlineData]).self) { group in
                for asset in assets {
                    group.addTask {
                        let symbol = asset.key.components(separatedBy: ":").dropFirst().joined(separator: ":")
                        let bars =
                            (try? await DataSourceFactory.shared.service(for: asset.source).fetchKlines(
                                symbol: symbol, interval: "1d", limit: 2000)) ?? []
                        return (asset.key, bars)
                    }
                }
                for await (key, bars) in group { histories[key] = bars }
            }
        }
        var points: [PortfolioSnapshot] = []
        var cursor = calendar.startOfDay(for: start)
        while cursor <= end {
            let historicalQuotes = Dictionary(
                assets.compactMap { asset -> (String, PortfolioQuote)? in
                    guard let bar = histories[asset.key]?.last(where: { $0.openTime <= cursor }),
                        cursor.timeIntervalSince(bar.openTime) <= 3 * 86_400
                    else { return nil }
                    return (asset.key, PortfolioQuote(price: Decimal(bar.closePrice), timestamp: bar.openTime))
                }, uniquingKeysWith: { existing, _ in existing })
            let holdingValues = try? PortfolioAccountingEngine.holdings(
                transactions: snapshot.transactions, portfolioIDs: [portfolio.id], through: cursor,
                quotes: historicalQuotes)
            let incomplete = holdingValues?.contains { $0.currentPrice == nil } ?? true
            let value = holdingValues?.compactMap(\.currentValue).reduce(0, +) ?? 0
            let cost = holdingValues?.reduce(0) { $0 + $1.costBasis } ?? 0
            let realized = holdingValues?.reduce(0) { $0 + $1.realizedPnL } ?? 0
            points.append(
                .init(
                    portfolioID: portfolio.id, timestamp: cursor, value: value,
                    netContributions: PortfolioAccountingEngine.netContributions(
                        snapshot.transactions, portfolioIDs: [portfolio.id], through: cursor),
                    realizedPnL: realized, unrealizedPnL: value - cost, isComplete: !incomplete))
            cursor = calendar.date(byAdding: .day, value: 1, to: cursor)!
        }
        do {
            try await ledger.storeSnapshots(points, for: portfolio.id, from: start)
            await refresh()
        } catch { lastError = error.localizedDescription }
    }

    nonisolated static func historyRebuildStart(
        for portfolioID: UUID, in snapshot: PortfolioLedgerSnapshot,
        today: Date, calendar: Calendar = Calendar(identifier: .gregorian)
    ) -> Date? {
        let today = calendar.startOfDay(for: today)
        if let invalidated = snapshot.invalidatedAfter[portfolioID] {
            return min(calendar.startOfDay(for: invalidated), today)
        }
        let existing = snapshot.historicalSnapshots.filter { $0.portfolioID == portfolioID }
        if let latest = existing.map(\.timestamp).max() {
            let next = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: latest))!
            return next <= today ? next : nil
        }
        return snapshot.transactions.filter { $0.portfolioID == portfolioID }
            .map { calendar.startOfDay(for: $0.timestamp) }.min()
    }

    private func derivedState(for ids: Set<UUID>) -> DerivedState {
        if let cached = derivedStates[ids] { return cached }
        let transactions = snapshot.transactions.filter { ids.contains($0.portfolioID) }
        let reportingCurrencies = Set(activePortfolios.filter { ids.contains($0.id) }.map(\.baseCurrency))
        let usableQuotes = reportingCurrencies == [.USD] ? quotes : [:]
        let holdings =
            (try? PortfolioAccountingEngine.holdings(
                transactions: snapshot.transactions, portfolioIDs: ids, quotes: usableQuotes)) ?? []
        let values = snapshot.historicalSnapshots.filter { ids.contains($0.portfolioID) }
        let history: [PortfolioSnapshot]
        if ids.count == 1 {
            history = values.sorted { $0.timestamp < $1.timestamp }
        } else {
            history = Self.aggregateHistory(values)
        }
        let state = DerivedState(
            transactions: transactions, holdings: holdings, history: history,
            totalValue: holdings.compactMap(\.currentValue).reduce(0, +),
            totalRealizedPnL: holdings.reduce(0) { $0 + $1.realizedPnL },
            totalUnrealizedPnL: holdings.compactMap(\.unrealizedPnL).reduce(0, +),
            unpricedAssetCount: holdings.filter { $0.currentPrice == nil }.count)
        derivedStates[ids] = state
        return state
    }

    nonisolated static func aggregateHistory(
        _ values: [PortfolioSnapshot],
        calendar: Calendar = Calendar(identifier: .gregorian)
    ) -> [PortfolioSnapshot] {
        let aggregateID = UUID(uuidString: "00000000-0000-0000-0000-000000000000")!
        return Dictionary(grouping: values, by: { calendar.startOfDay(for: $0.timestamp) }).map { date, points in
            PortfolioSnapshot(
                portfolioID: aggregateID, timestamp: date,
                value: points.reduce(0) { $0 + $1.value },
                netContributions: points.reduce(0) { $0 + $1.netContributions },
                realizedPnL: points.reduce(0) { $0 + $1.realizedPnL },
                unrealizedPnL: points.reduce(0) { $0 + $1.unrealizedPnL },
                isComplete: points.allSatisfy(\.isComplete))
        }.sorted { $0.timestamp < $1.timestamp }
    }

    nonisolated static func needsInitialQuoteLoad(
        snapshot: PortfolioLedgerSnapshot,
        quotes: [String: PortfolioQuote]
    ) -> Bool {
        let selectedIDs: Set<UUID>
        if let selected = snapshot.selectedPortfolioID {
            selectedIDs = [selected]
        } else {
            selectedIDs = Set(snapshot.portfolios.filter { !$0.isArchived }.map(\.id))
        }
        return quoteEligibleAssets(in: snapshot, portfolioIDs: selectedIDs).contains { quotes[$0.key] == nil }
    }

    nonisolated static func quoteEligibleAssets(
        in snapshot: PortfolioLedgerSnapshot,
        portfolioIDs: Set<UUID>
    ) -> [PortfolioAsset] {
        let portfolios = snapshot.portfolios.filter { portfolioIDs.contains($0.id) }
        guard !portfolios.isEmpty, portfolios.allSatisfy({ $0.baseCurrency == .USD }) else { return [] }
        let transactions = snapshot.transactions.filter { portfolioIDs.contains($0.portfolioID) }
        return PortfolioAccountingEngine.uniqueAssets(in: transactions).filter { $0.quoteCurrency == .USD }
    }

    private func pruneQuotes() {
        let referenced = Set(PortfolioAccountingEngine.uniqueAssets(in: snapshot.transactions).map(\.key))
        let pruned = quotes.filter { referenced.contains($0.key) }
        if pruned != quotes {
            quotes = pruned
            derivedStates.removeAll(keepingCapacity: true)
            quoteStore.save(pruned)
        }
    }

    private func perform(_ operation: @escaping () async throws -> Void) { Task { _ = await result(operation) } }
    private func result(_ operation: () async throws -> Void) async -> Bool {
        do {
            try await operation()
            await refresh()
            return true
        } catch {
            lastError = error.localizedDescription
            return false
        }
    }
}

private extension PortfolioSelection {
    var id: UUID? {
        if case .portfolio(let id) = self { return id }
        return nil
    }
}
