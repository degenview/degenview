import Foundation
import SwiftUI

@MainActor
final class PortfolioStore: ObservableObject {
    static let shared = PortfolioStore()

    @Published private(set) var snapshot: PortfolioLedgerSnapshot
    @Published private(set) var quotes: [String: PortfolioQuote] = [:]
    @Published private(set) var isRefreshing = false
    @Published var lastError: String?
    @AppStorage("portfolioPrivacyMode") var privacyMode = false

    let ledger: PortfolioLedger

    private init() {
        let initial = JSONStore<PortfolioLedgerSnapshot>(filename: "portfolios.json").load() ?? .empty
        snapshot = initial
        ledger = PortfolioLedger(snapshot: initial, persist: { value in
            JSONStore<PortfolioLedgerSnapshot>(filename: "portfolios.json").save(value)
        })
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
    var transactions: [PortfolioTransaction] { snapshot.transactions.filter { selectedPortfolioIDs.contains($0.portfolioID) } }
    private var valuationQuotes: [String: PortfolioQuote] {
        let reportingCurrencies = Set(activePortfolios.filter { selectedPortfolioIDs.contains($0.id) }.map(\.baseCurrency))
        return reportingCurrencies == [.USD] ? quotes : [:]
    }
    var holdings: [PortfolioHolding] {
        (try? PortfolioAccountingEngine.holdings(transactions: snapshot.transactions,
            portfolioIDs: selectedPortfolioIDs, quotes: valuationQuotes)) ?? []
    }
    var totalValue: Decimal { holdings.compactMap(\.currentValue).reduce(0, +) }
    var totalRealizedPnL: Decimal { holdings.reduce(0) { $0 + $1.realizedPnL } }
    var totalUnrealizedPnL: Decimal { holdings.compactMap(\.unrealizedPnL).reduce(0, +) }
    var totalPnL: Decimal { totalRealizedPnL + totalUnrealizedPnL }
    var unpricedAssetCount: Int { holdings.filter { $0.currentPrice == nil }.count }

    func refresh() async { snapshot = await ledger.snapshot() }
    func select(_ selection: PortfolioSelection) { perform { try await self.ledger.select(selection.id) } }
    func create(name: String, currency: PortfolioCurrency) { perform { _ = try await self.ledger.createPortfolio(name: name, currency: currency) } }
    func update(_ portfolio: Portfolio) { perform { try await self.ledger.updatePortfolio(portfolio) } }
    func duplicate(_ id: UUID) { perform { _ = try await self.ledger.duplicatePortfolio(id) } }
    func delete(_ id: UUID) { perform { try await self.ledger.deletePortfolio(id) } }
    func reorder(from: IndexSet, to: Int) { perform { try await self.ledger.reorder(from: from, to: to) } }
    func add(_ transaction: PortfolioTransaction) async -> Bool { await result { try await self.ledger.add(transaction) } }
    func update(_ transaction: PortfolioTransaction) async -> Bool { await result { try await self.ledger.update(transaction) } }
    func deleteTransaction(_ id: UUID) { perform { try await self.ledger.deleteTransaction(id) } }
    func remapAsset(from sourceKey: String, to destination: PortfolioAsset,
                    portfolioIDs: Set<UUID>) async throws {
        do {
            try await ledger.remapAsset(from: sourceKey, to: destination, portfolioIDs: portfolioIDs)
            await refresh()
        } catch {
            lastError = error.localizedDescription
            throw error
        }
    }
    func importTransactions(_ values: [PortfolioTransaction]) async -> Bool { await result { try await self.ledger.importTransactions(values) } }

    func refreshQuotes() async {
        let assets = PortfolioAccountingEngine.uniqueAssets(in: transactions)
        guard !assets.isEmpty else { return }
        isRefreshing = true; defer { isRefreshing = false }
        await withTaskGroup(of: (String, PortfolioQuote?).self) { group in
            for asset in assets {
                group.addTask {
                    guard asset.quoteCurrency == .USD else { return (asset.key, nil) }
                    let service = DataSourceFactory.shared.service(for: asset.source)
                    guard let data = try? await service.fetchKlines(symbol: asset.key.components(separatedBy: ":").dropFirst().joined(separator: ":"), interval: "1h", limit: 25),
                          let latest = data.last else { return (asset.key, nil) }
                    let prior = data.last(where: { latest.openTime.timeIntervalSince($0.openTime) >= 23 * 3600 })
                    return (asset.key, PortfolioQuote(price: Decimal(latest.closePrice),
                        previousDayPrice: prior.map { Decimal($0.closePrice) }, timestamp: Date()))
                }
            }
            for await (key, quote) in group { if let quote { quotes[key] = quote } }
        }
    }

    /// Rebuilds only the invalidated suffix. Each daily point uses holdings that existed then
    /// and a nearest prior real close; missing observations produce an incomplete snapshot.
    func rebuildHistory() async {
        let portfolios = selectedPortfolio.map { [$0] } ?? activePortfolios
        for portfolio in portfolios { await rebuildHistory(for: portfolio) }
    }

    private func rebuildHistory(for portfolio: Portfolio) async {
        let start = snapshot.invalidatedAfter[portfolio.id]
            ?? snapshot.transactions.filter { $0.portfolioID == portfolio.id }.map(\.timestamp).min()
        guard let start else { return }
        let calendar = Calendar(identifier: .gregorian)
        let end = Date()
        let assets = PortfolioAccountingEngine.uniqueAssets(
            in: snapshot.transactions.filter { $0.portfolioID == portfolio.id }
        )
        var histories: [String: [KlineData]] = [:]
        if portfolio.baseCurrency == .USD { await withTaskGroup(of: (String, [KlineData]).self) { group in
            for asset in assets { group.addTask {
                let symbol = asset.key.components(separatedBy: ":").dropFirst().joined(separator: ":")
                let bars = (try? await DataSourceFactory.shared.service(for: asset.source).fetchKlines(symbol: symbol, interval: "1d", limit: 2000)) ?? []
                return (asset.key, bars)
            } }
            for await (key, bars) in group { histories[key] = bars }
        } }
        var points: [PortfolioSnapshot] = [], cursor = calendar.startOfDay(for: start)
        while cursor <= end {
            let historicalQuotes = Dictionary(assets.compactMap { asset -> (String, PortfolioQuote)? in
                guard let bar = histories[asset.key]?.last(where: { $0.openTime <= cursor }), cursor.timeIntervalSince(bar.openTime) <= 3 * 86_400 else { return nil }
                return (asset.key, PortfolioQuote(price: Decimal(bar.closePrice), timestamp: bar.openTime))
            }, uniquingKeysWith: { existing, _ in existing })
            let holdingValues = try? PortfolioAccountingEngine.holdings(transactions: snapshot.transactions, portfolioIDs: [portfolio.id], through: cursor, quotes: historicalQuotes)
            let incomplete = holdingValues?.contains { $0.currentPrice == nil } ?? true
            let value = holdingValues?.compactMap(\.currentValue).reduce(0, +) ?? 0
            let cost = holdingValues?.reduce(0) { $0 + $1.costBasis } ?? 0
            let realized = holdingValues?.reduce(0) { $0 + $1.realizedPnL } ?? 0
            points.append(.init(portfolioID: portfolio.id, timestamp: cursor, value: value,
                netContributions: PortfolioAccountingEngine.netContributions(snapshot.transactions, portfolioIDs: [portfolio.id], through: cursor),
                realizedPnL: realized, unrealizedPnL: value - cost, isComplete: !incomplete))
            cursor = calendar.date(byAdding: .day, value: 1, to: cursor)!
        }
        do { try await ledger.storeSnapshots(points, for: portfolio.id, from: start); await refresh() } catch { lastError = error.localizedDescription }
    }

    private func perform(_ operation: @escaping () async throws -> Void) { Task { _ = await result(operation) } }
    private func result(_ operation: () async throws -> Void) async -> Bool {
        do { try await operation(); await refresh(); return true } catch { lastError = error.localizedDescription; return false }
    }
}

private extension PortfolioSelection { var id: UUID? { if case .portfolio(let id) = self { return id }; return nil } }
