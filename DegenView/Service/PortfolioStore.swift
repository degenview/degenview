import Foundation
import SwiftUI

@MainActor
final class PortfolioStore: ObservableObject {
    static let shared = PortfolioStore()

    @Published private(set) var snapshot: PortfolioLedgerSnapshot
    @Published private(set) var quotes: [String: PortfolioQuote]
    @Published private(set) var isRefreshing = false
    /// True while daily history is being rebuilt. The chart has its own loading state; the value doesn't wait on it.
    @Published private(set) var isLoadingHistory = false
    @Published private(set) var isLoadingInitialValues: Bool
    @Published private(set) var reportingCurrency: PortfolioCurrency = .USD
    @Published private(set) var isChangingReportingCurrency = false
    @Published private(set) var reportingConversionProgress: Double?
    @Published private(set) var usesCachedExchangeRates = false
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
    private var derivedStates: [Set<UUID>: DerivedState] = [:] {
        didSet { if derivedStates.isEmpty { intradayStates.removeAll() } }
    }
    /// Recent fine-grained candles per asset key, for the 1D chart. In memory only.
    struct IntradayBars {
        var bars: [KlineData]
        var seconds: TimeInterval
        var fetched: Date
    }
    private var intradayBars: [String: IntradayBars] = [:]
    private var intradayInFlight: Set<String> = []
    private var intradayStates: [Set<UUID>: (bucket: Int, points: [PortfolioSnapshot])] = [:]
    nonisolated static let intradayStep: TimeInterval = 1_800
    private struct ReportingPreferences: Codable { var currencies: [String: PortfolioCurrency] = [:] }
    static let reportingPreferencesKey = "portfolio.reportingCurrencies"
    private let database: AppDatabase
    private var reportingPreferences: ReportingPreferences
    private var convertedTransactions: [UUID: PortfolioTransaction] = [:]
    private var convertedQuotes: [UUID: [String: PortfolioQuote]] = [:]
    private var convertedHistory: [String: PortfolioSnapshot] = [:]
    private var baseToReportingRates: [UUID: Decimal] = [:]
    private struct ProjectionKey: Hashable {
        let portfolioIDs: [UUID]
        let currency: PortfolioCurrency
        let ledgerGeneration: Int
        let quoteGeneration: Int
    }
    private var projectionCache: [ProjectionKey: Projection] = [:]
    private var projectionCacheOrder: [ProjectionKey] = []
    private var ledgerGeneration = 0
    private var quoteGeneration = 0
    private let projectionCacheLimit = 12
    private let fxService: any FXRateProviding
    private let initialReportingCurrency: PortfolioCurrency
    private var hasInitialized = false
    @Published private(set) var isInitializing = false
    private var historyTasks: [UUID: Task<Bool, Never>] = [:]
    private var historyRebuildsInFlight = 0
    private var quoteTasks: [String: Task<PortfolioQuote?, Never>] = [:]
    private let quoteFreshness: TimeInterval = 5 * 60
    private let quoteStore: JSONStore<[String: PortfolioQuote]>
    private let candleStore: PortfolioCandleStore
    private let showsCachedValues: Bool
    private let quoteFetcher: PortfolioQuoteFetcher

    init(
        initialSnapshot: PortfolioLedgerSnapshot? = nil, initialQuotes suppliedQuotes: [String: PortfolioQuote]? = nil,
        fxService: any FXRateProviding = FXRateService.shared, database: AppDatabase = .shared,
        storageDirectory: URL = AppSupport.directory, candleStore: PortfolioCandleStore? = nil,
        quoteFetcher: PortfolioQuoteFetcher = .live
    ) {
        let quoteStore = JSONStore<[String: PortfolioQuote]>(
            filename: "portfolio_quotes.json", directory: storageDirectory)
        var persisted: PortfolioLedgerSnapshot?
        var canPersist = true
        if initialSnapshot == nil {
            do {
                persisted = try database.portfolioLedger()
            } catch {
                // Never let an empty in-memory ledger overwrite one that failed to decode.
                canPersist = false
                #if DEBUG
                    print("[PortfolioStore] Ledger unreadable; disabling writes: \(error.localizedDescription)")
                #endif
            }
        }
        let initial = initialSnapshot ?? persisted ?? .empty
        let loadedQuotes = suppliedQuotes ?? quoteStore.load() ?? [:]
        let referencedKeys = Set(PortfolioAccountingEngine.uniqueAssets(in: initial.transactions).map(\.key))
        let initialQuotes = loadedQuotes.filter { referencedKeys.contains($0.key) }
        self.fxService = fxService
        self.quoteStore = quoteStore
        self.database = database
        self.candleStore = candleStore ?? PortfolioCandleStore(database: database)
        self.quoteFetcher = quoteFetcher
        snapshot = initial
        quotes = initialQuotes
        reportingPreferences =
            database.setting(ReportingPreferences.self, key: Self.reportingPreferencesKey) ?? ReportingPreferences()
        let preferredCurrency = Self.preferredCurrency(
            selectionID: initial.selectedPortfolioID, snapshot: initial, preferences: reportingPreferences)
        let nativeCurrency = Self.nativeCurrency(selectionID: initial.selectedPortfolioID, snapshot: initial)
        initialReportingCurrency = preferredCurrency
        reportingCurrency = nativeCurrency
        // Persisted quotes can paint real numbers before any network call, but only if every
        // asset has one (an unpriced asset would read as 0) and the reporting currency is the
        // stored one (otherwise the value would flash in the wrong currency).
        showsCachedValues =
            !Self.needsInitialQuoteLoad(snapshot: initial, quotes: initialQuotes) && preferredCurrency == nativeCurrency
        isLoadingInitialValues = !initial.portfolios.isEmpty
        ledger = PortfolioLedger(
            snapshot: initial,
            persist: canPersist ? { value in try database.savePortfolioLedger(value) } : nil)
        if initialQuotes != loadedQuotes {
            quoteStore.save(initialQuotes)
        }
    }

    /// Background work is running behind values that are already on screen.
    var isUpdating: Bool { isInitializing || isRefreshing }

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
    /// Unprojected ledger records for editing and CSV export.
    var ledgerTransactions: [PortfolioTransaction] {
        snapshot.transactions.filter { selectedPortfolioIDs.contains($0.portfolioID) }
    }
    var ledgerHoldings: [PortfolioHolding] {
        let currencies = Set(activePortfolios.filter { selectedPortfolioIDs.contains($0.id) }.map(\.baseCurrency))
        let usableQuotes = currencies == [.USD] ? quotes : [:]
        return
            (try? PortfolioAccountingEngine.holdings(
                transactions: snapshot.transactions, portfolioIDs: selectedPortfolioIDs, quotes: usableQuotes)) ?? []
    }
    var ledgerHistory: [PortfolioSnapshot] {
        let values = snapshot.historicalSnapshots.filter { selectedPortfolioIDs.contains($0.portfolioID) }
        return selectedPortfolioIDs.count == 1
            ? values.sorted { $0.timestamp < $1.timestamp } : Self.aggregateHistory(values)
    }
    var holdings: [PortfolioHolding] { derivedState(for: selectedPortfolioIDs).holdings }
    var totalValue: Decimal { derivedState(for: selectedPortfolioIDs).totalValue }
    var totalRealizedPnL: Decimal { derivedState(for: selectedPortfolioIDs).totalRealizedPnL }
    var totalUnrealizedPnL: Decimal { derivedState(for: selectedPortfolioIDs).totalUnrealizedPnL }
    var totalPnL: Decimal { totalRealizedPnL + totalUnrealizedPnL }
    var unpricedAssetCount: Int { derivedState(for: selectedPortfolioIDs).unpricedAssetCount }
    var marketValueCaption: String {
        let exchangeSuffix = usesCachedExchangeRates ? " · Cached exchange rates" : ""
        if unpricedAssetCount > 0 {
            return "+ \(unpricedAssetCount) unpriced asset\(unpricedAssetCount == 1 ? "" : "s")\(exchangeSuffix)"
        }
        let now = Date()
        let assets = Self.quoteEligibleAssets(in: snapshot, portfolioIDs: selectedPortfolioIDs)
        return assets.contains { asset in
            quotes[asset.key].map { now.timeIntervalSince($0.timestamp) >= quoteFreshness } ?? false
        } ? "Cached market value\(exchangeSuffix)" : "Live market value\(exchangeSuffix)"
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
        let contentChanged =
            latest.portfolios != snapshot.portfolios || latest.transactions != snapshot.transactions
            || latest.historicalSnapshots != snapshot.historicalSnapshots
            || latest.invalidatedAfter != snapshot.invalidatedAfter
        snapshot = latest
        if contentChanged {
            ledgerGeneration += 1
            discardObsoleteProjections()
        }
        derivedStates.removeAll(keepingCapacity: true)
        pruneQuotes()
    }

    func initialize() async {
        guard !hasInitialized else { return }
        hasInitialized = true
        isInitializing = true
        defer {
            isInitializing = false
            isLoadingInitialValues = false
        }
        await refresh()
        if showsCachedValues, initialReportingCurrency == reportingCurrency {
            // Values are read from the projection, so build it from the persisted quotes first;
            // without network work this is quick, and the screen can show numbers while the rest loads.
            await reloadReportingProjection()
            isLoadingInitialValues = false
        }
        // The value needs only current prices, so they go first and alone: history requests would
        // queue behind them on shared rate limiters. The chart is filled in afterwards.
        await refreshQuotes(for: selectedPortfolioIDs)
        if initialReportingCurrency == reportingCurrency {
            await reloadReportingProjection()
            isLoadingInitialValues = false
        } else {
            await selectReportingCurrency(initialReportingCurrency)
        }
        await rebuildHistory()
    }
    func select(_ selection: PortfolioSelection) {
        Task {
            do {
                try await ledger.select(selection.id)
                await refresh()
                await loadReportingCurrencyForSelection()
            } catch { lastError = error.localizedDescription }
        }
    }

    func selectReportingCurrency(_ currency: PortfolioCurrency) async {
        guard currency != reportingCurrency, !isChangingReportingCurrency else { return }
        let key = projectionKey(currency: currency)
        if let projection = projectionCache[key] {
            commit(projection)
            reportingCurrency = currency
            saveReportingPreference(currency)
            touchProjection(key)
            lastError = nil
            return
        }
        isChangingReportingCurrency = true
        reportingConversionProgress = 0
        defer {
            isChangingReportingCurrency = false
            reportingConversionProgress = nil
        }
        do {
            let projection = try await buildProjection(reportingCurrency: currency)
            commit(projection)
            cache(projection, for: key)
            reportingCurrency = currency
            saveReportingPreference(currency)
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    func reportingCurrency(for portfolioID: UUID?) -> PortfolioCurrency {
        Self.preferredCurrency(selectionID: portfolioID, snapshot: snapshot, preferences: reportingPreferences)
    }
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
            await reloadReportingProjection()
        } catch {
            lastError = error.localizedDescription
            throw error
        }
    }
    func importTransactions(_ values: [PortfolioTransaction]) async -> Bool {
        await result { try await self.ledger.importTransactions(values) }
    }

    func refreshQuotes() async {
        defer { if !isInitializing { isLoadingInitialValues = false } }
        await refreshQuotes(for: selectedPortfolioIDs)
        await reloadReportingProjection()
    }

    func refreshQuotes(forPortfolioID id: UUID?) async {
        let ids = id.map { Set([$0]) } ?? Set(activePortfolios.map(\.id))
        await refreshQuotes(for: ids)
        if !selectedPortfolioIDs.isDisjoint(with: ids) { await reloadReportingProjection() }
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
        let stale = assets.filter { asset in
            quotes[asset.key].map { now.timeIntervalSince($0.timestamp) >= quoteFreshness } ?? true
        }
        // One fetch prices every asset not already in flight; each asset's task just picks its
        // own quote out of it, so a concurrent caller shares the request instead of repeating it.
        let unclaimed = stale.filter { quoteTasks[$0.key] == nil }
        if !unclaimed.isEmpty {
            let fetcher = quoteFetcher
            let batch = Task { await fetcher.quotes(for: unclaimed) }
            for asset in unclaimed { quoteTasks[asset.key] = Task { await batch.value[asset.key] } }
        }
        let pending = stale.compactMap { asset in quoteTasks[asset.key].map { (asset.key, $0) } }
        var updates: [String: PortfolioQuote] = [:]
        await withTaskGroup(of: (String, PortfolioQuote?).self) { group in
            for (key, task) in pending { group.addTask { (key, await task.value) } }
            for await (key, quote) in group { if let quote { updates[key] = quote } }
        }
        for asset in stale { quoteTasks[asset.key] = nil }
        if !updates.isEmpty {
            var merged = quotes
            merged.merge(updates) { _, new in new }
            if merged != quotes {
                quotes = merged
                quoteGeneration += 1
                discardObsoleteProjections()
                derivedStates.removeAll(keepingCapacity: true)
                quoteStore.save(merged)
            }
        }
        pruneQuotes()
    }

    /// Rebuilds only the invalidated suffix. Each daily point uses holdings that existed then
    /// and a nearest prior real close; missing observations produce an incomplete snapshot.
    func rebuildHistory() async {
        await rebuildHistory(selectedPortfolio.map { [$0] } ?? activePortfolios)
    }

    func rebuildHistory(forPortfolioID id: UUID?) async {
        await rebuildHistory(
            id.flatMap { target in activePortfolios.first { $0.id == target } }.map { [$0] } ?? activePortfolios)
    }

    /// Portfolios rebuild side by side; the projection reloads once afterwards so two rebuilds
    /// never race to commit one built from older data.
    private func rebuildHistory(_ portfolios: [Portfolio]) async {
        var changed: Set<UUID> = []
        historyRebuildsInFlight += 1
        isLoadingHistory = true
        defer {
            historyRebuildsInFlight -= 1
            isLoadingHistory = historyRebuildsInFlight > 0
        }
        await withTaskGroup(of: (UUID, Bool).self) { group in
            for portfolio in portfolios {
                group.addTask { (portfolio.id, await self.rebuildHistory(for: portfolio)) }
            }
            for await (id, didChange) in group where didChange { changed.insert(id) }
        }
        if !selectedPortfolioIDs.isDisjoint(with: changed) { await reloadReportingProjection() }
    }

    /// True when new points were stored.
    private func rebuildHistory(for portfolio: Portfolio) async -> Bool {
        if let task = historyTasks[portfolio.id] { return await task.value }
        guard let start = Self.historyRebuildStart(for: portfolio.id, in: snapshot, today: Date()) else {
            return false
        }
        let task = Task { [weak self] () -> Bool in
            guard let self else { return false }
            return await self.performHistoryRebuild(for: portfolio, start: start)
        }
        historyTasks[portfolio.id] = task
        let didChange = await task.value
        historyTasks[portfolio.id] = nil
        return didChange
    }

    private func performHistoryRebuild(for portfolio: Portfolio, start: Date) async -> Bool {
        let end = Date()
        let transactions = snapshot.transactions.filter { $0.portfolioID == portfolio.id }
        let assets = PortfolioAccountingEngine.uniqueAssets(in: transactions)
        var histories: [String: [KlineData]] = [:]
        if portfolio.baseCurrency == .USD {
            let candleStore = candleStore
            await withTaskGroup(of: (String, [KlineData]).self) { group in
                for asset in assets {
                    group.addTask { (asset.key, await candleStore.dailyBars(for: asset, from: start)) }
                }
                for await (key, bars) in group { histories[key] = bars }
            }
        }
        // One step per day, each replaying the ledger: too much to run on the main actor.
        let fetched = histories
        let points = await Task.detached(priority: .userInitiated) {
            Self.buildSnapshots(
                portfolioID: portfolio.id, transactions: transactions, assets: assets, histories: fetched,
                start: start, end: end)
        }.value
        do {
            try await ledger.storeSnapshots(points, for: portfolio.id, from: start)
            await refresh()
            return true
        } catch {
            lastError = error.localizedDescription
            return false
        }
    }

    /// One snapshot per day from `start` through `end`, priced from the nearest prior daily
    /// bar no more than three days old. `transactions` must already be the portfolio's own.
    nonisolated static func buildSnapshots(
        portfolioID: UUID, transactions: [PortfolioTransaction], assets: [PortfolioAsset],
        histories: [String: [KlineData]], start: Date, end: Date,
        calendar: Calendar = Calendar(identifier: .gregorian)
    ) -> [PortfolioSnapshot] {
        let bars = histories.mapValues { $0.sorted { $0.openTime < $1.openTime } }
        var points: [PortfolioSnapshot] = []
        var cursor = calendar.startOfDay(for: start)
        while cursor <= end {
            let historicalQuotes = Dictionary(
                assets.compactMap { asset -> (String, PortfolioQuote)? in
                    guard let bar = lastBar(in: bars[asset.key] ?? [], atOrBefore: cursor),
                        cursor.timeIntervalSince(bar.openTime) <= 3 * 86_400
                    else { return nil }
                    return (asset.key, PortfolioQuote(price: Decimal(bar.closePrice), timestamp: bar.openTime))
                }, uniquingKeysWith: { existing, _ in existing })
            let holdingValues = try? PortfolioAccountingEngine.holdings(
                transactions: transactions, portfolioIDs: [portfolioID], through: cursor,
                quotes: historicalQuotes)
            let incomplete = holdingValues?.contains { $0.currentPrice == nil } ?? true
            let value = holdingValues?.compactMap(\.currentValue).reduce(0, +) ?? 0
            let cost = holdingValues?.reduce(0) { $0 + $1.costBasis } ?? 0
            let realized = holdingValues?.reduce(0) { $0 + $1.realizedPnL } ?? 0
            points.append(
                .init(
                    portfolioID: portfolioID, timestamp: cursor, value: value,
                    netContributions: PortfolioAccountingEngine.netContributions(
                        transactions, portfolioIDs: [portfolioID], through: cursor),
                    realizedPnL: realized, unrealizedPnL: value - cost, isComplete: !incomplete))
            cursor = calendar.date(byAdding: .day, value: 1, to: cursor)!
        }
        return points
    }

    /// Binary search over bars sorted by open time.
    nonisolated private static func lastBar(in bars: [KlineData], atOrBefore date: Date) -> KlineData? {
        var low = 0
        var high = bars.count
        while low < high {
            let mid = (low + high) / 2
            if bars[mid].openTime <= date { low = mid + 1 } else { high = mid }
        }
        return low > 0 ? bars[low - 1] : nil
    }

    /// Fetches the last day of fine candles for the assets the 1D chart replays. Alpaca has
    /// no 15-minute timeframe, so it is sampled from hourly bars.
    func refreshIntraday(forPortfolioID id: UUID?) async {
        let portfolios = activePortfolios.filter { ($0.id == id || id == nil) && $0.baseCurrency == .USD }
        let now = Date()
        let assets = PortfolioAccountingEngine.uniqueAssets(
            in: snapshot.transactions.filter { transaction in portfolios.contains { $0.id == transaction.portfolioID } }
        ).filter { asset in
            asset.quoteCurrency == .USD && !intradayInFlight.contains(asset.key)
                && intradayBars[asset.key].map { now.timeIntervalSince($0.fetched) >= quoteFreshness } ?? true
        }
        guard !assets.isEmpty else { return }
        intradayInFlight.formUnion(assets.map(\.key))
        var fetched: [String: IntradayBars] = [:]
        await withTaskGroup(of: (String, IntradayBars?).self) { group in
            for asset in assets {
                group.addTask {
                    let hourly = asset.source == .alpaca
                    let symbol = asset.key.components(separatedBy: ":").dropFirst().joined(separator: ":")
                    guard
                        let bars = try? await DataSourceFactory.shared.service(for: asset.source).fetchKlines(
                            symbol: symbol, interval: hourly ? "1h" : "15m", limit: hourly ? 48 : 120),
                        !bars.isEmpty
                    else { return (asset.key, nil) }
                    return (asset.key, IntradayBars(bars: bars, seconds: hourly ? 3_600 : 900, fetched: Date()))
                }
            }
            for await (key, result) in group { if let result { fetched[key] = result } }
        }
        intradayInFlight.subtract(assets.map(\.key))
        guard !fetched.isEmpty else { return }
        intradayBars.merge(fetched) { _, new in new }
        derivedStates.removeAll(keepingCapacity: true)
        objectWillChange.send()
    }

    /// The last 24 hours at 30-minute spacing, in the reporting currency. Empty when the
    /// selection can't be replayed (non-USD portfolios, no candles yet) so callers fall back
    /// to the daily history.
    func intradayHistory(for portfolioID: UUID?, now: Date = Date()) -> [PortfolioSnapshot] {
        let portfolios = activePortfolios.filter { $0.id == portfolioID || portfolioID == nil }
        guard !portfolios.isEmpty, portfolios.allSatisfy({ $0.baseCurrency == .USD }) else { return [] }
        let ids = Set(portfolios.map(\.id))
        let bucket = Int(now.timeIntervalSince1970 / Self.intradayStep)
        if let cached = intradayStates[ids], cached.bucket == bucket { return cached.points }
        let transactions = derivedState(for: ids).transactions
        let series = portfolios.map {
            Self.intradaySnapshots(
                transactions: transactions, portfolioID: $0.id, bars: intradayBars,
                rateToReporting: baseToReportingRates[$0.id] ?? 1, now: now)
        }
        let points = series.count == 1 ? series[0] : Self.sumIntraday(series)
        intradayStates[ids] = (bucket, points)
        return points
    }

    /// Portfolio value at every `step` boundary of the trailing `window`, replaying holdings
    /// at each slot against the close of the last candle that had finished by then. A candle
    /// older than three days is treated as no quote, like the daily rebuild.
    nonisolated static func intradaySnapshots(
        transactions: [PortfolioTransaction], portfolioID: UUID, bars: [String: IntradayBars],
        rateToReporting: Decimal, now: Date, step: TimeInterval = intradayStep, window: TimeInterval = 86_400
    ) -> [PortfolioSnapshot] {
        let own = transactions.filter { $0.portfolioID == portfolioID }
        guard let first = own.map(\.timestamp).min() else { return [] }
        let assets = PortfolioAccountingEngine.uniqueAssets(in: own)
        let end = (now.timeIntervalSince1970 / step).rounded(.down) * step
        var points: [PortfolioSnapshot] = []
        var slotTime = end - window
        while slotTime <= end {
            defer { slotTime += step }
            let slot = Date(timeIntervalSince1970: slotTime)
            guard slot >= first else { continue }
            let quotes = Dictionary(
                assets.compactMap { asset -> (String, PortfolioQuote)? in
                    guard let series = bars[asset.key],
                        let bar = series.bars.last(where: { $0.openTime.addingTimeInterval(series.seconds) <= slot }),
                        slot.timeIntervalSince(bar.openTime) <= 3 * 86_400
                    else { return nil }
                    return (
                        asset.key,
                        PortfolioQuote(price: Decimal(bar.closePrice) * rateToReporting, timestamp: bar.openTime)
                    )
                }, uniquingKeysWith: { existing, _ in existing })
            let holdings = try? PortfolioAccountingEngine.holdings(
                transactions: own, portfolioIDs: [portfolioID], through: slot, quotes: quotes)
            let value = holdings?.compactMap(\.currentValue).reduce(0, +) ?? 0
            let cost = holdings?.reduce(0) { $0 + $1.costBasis } ?? 0
            points.append(
                .init(
                    portfolioID: portfolioID, timestamp: slot, value: value,
                    netContributions: PortfolioAccountingEngine.netContributions(
                        own, portfolioIDs: [portfolioID], through: slot),
                    realizedPnL: holdings?.reduce(0) { $0 + $1.realizedPnL } ?? 0, unrealizedPnL: value - cost,
                    isComplete: !(holdings?.contains { $0.currentPrice == nil } ?? true)))
        }
        return points
    }

    nonisolated static func sumIntraday(_ series: [[PortfolioSnapshot]]) -> [PortfolioSnapshot] {
        let aggregateID = UUID(uuidString: "00000000-0000-0000-0000-000000000000")!
        return Dictionary(grouping: series.flatMap { $0 }, by: \.timestamp).map { date, points in
            PortfolioSnapshot(
                portfolioID: aggregateID, timestamp: date,
                value: points.reduce(0) { $0 + $1.value },
                netContributions: points.reduce(0) { $0 + $1.netContributions },
                realizedPnL: points.reduce(0) { $0 + $1.realizedPnL },
                unrealizedPnL: points.reduce(0) { $0 + $1.unrealizedPnL },
                isComplete: points.allSatisfy(\.isComplete))
        }.sorted { $0.timestamp < $1.timestamp }
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
        let rawTransactions = snapshot.transactions.filter { ids.contains($0.portfolioID) }
        let transactions = rawTransactions.map { convertedTransactions[$0.id] ?? $0 }
        var mergedQuotes: [String: PortfolioQuote] = [:]
        for id in ids { mergedQuotes.merge(convertedQuotes[id] ?? [:]) { existing, _ in existing } }
        let holdings =
            (try? PortfolioAccountingEngine.holdings(
                transactions: transactions, portfolioIDs: ids, quotes: mergedQuotes)) ?? []
        let values = snapshot.historicalSnapshots.filter { ids.contains($0.portfolioID) }.map {
            convertedHistory[$0.id] ?? $0
        }
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

    func reportingPrice(for transaction: PortfolioTransaction) -> Decimal? {
        convertedTransactions[transaction.id]?.price
    }

    private struct Projection {
        let transactions: [UUID: PortfolioTransaction]
        let quotes: [UUID: [String: PortfolioQuote]]
        let history: [String: PortfolioSnapshot]
        let rates: [UUID: Decimal]
        let usesCache: Bool
    }

    private func projectionKey(currency: PortfolioCurrency) -> ProjectionKey {
        ProjectionKey(
            portfolioIDs: selectedPortfolioIDs.sorted { $0.uuidString < $1.uuidString }, currency: currency,
            ledgerGeneration: ledgerGeneration, quoteGeneration: quoteGeneration)
    }

    private func cache(_ projection: Projection, for key: ProjectionKey) {
        projectionCache[key] = projection
        touchProjection(key)
        while projectionCacheOrder.count > projectionCacheLimit {
            projectionCache.removeValue(forKey: projectionCacheOrder.removeFirst())
        }
    }

    private func touchProjection(_ key: ProjectionKey) {
        projectionCacheOrder.removeAll { $0 == key }
        projectionCacheOrder.append(key)
    }

    private func discardObsoleteProjections() {
        projectionCache = projectionCache.filter {
            $0.key.ledgerGeneration == ledgerGeneration && $0.key.quoteGeneration == quoteGeneration
        }
        projectionCacheOrder.removeAll { projectionCache[$0] == nil }
    }

    private func saveReportingPreference(_ currency: PortfolioCurrency) {
        reportingPreferences.currencies[Self.preferenceKey(snapshot.selectedPortfolioID)] = currency
        database.setSetting(reportingPreferences, key: Self.reportingPreferencesKey)
    }

    private func loadReportingCurrencyForSelection() async {
        let desired = Self.preferredCurrency(
            selectionID: snapshot.selectedPortfolioID, snapshot: snapshot, preferences: reportingPreferences)
        if desired == reportingCurrency {
            await reloadReportingProjection()
        } else {
            await selectReportingCurrency(desired)
        }
    }

    private func reloadReportingProjection() async {
        let key = projectionKey(currency: reportingCurrency)
        if let projection = projectionCache[key] {
            commit(projection)
            touchProjection(key)
            lastError = nil
            return
        }
        isChangingReportingCurrency = true
        reportingConversionProgress = 0
        defer {
            isChangingReportingCurrency = false
            reportingConversionProgress = nil
        }
        do {
            let projection = try await buildProjection(reportingCurrency: reportingCurrency)
            commit(projection)
            cache(projection, for: key)
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    private func commit(_ projection: Projection) {
        convertedTransactions = projection.transactions
        convertedQuotes = projection.quotes
        convertedHistory = projection.history
        baseToReportingRates = projection.rates
        usesCachedExchangeRates = projection.usesCache
        derivedStates.removeAll(keepingCapacity: true)
        objectWillChange.send()
    }

    private func buildProjection(reportingCurrency: PortfolioCurrency) async throws -> Projection {
        var projectedTransactions: [UUID: PortfolioTransaction] = [:]
        var projectedQuotes: [UUID: [String: PortfolioQuote]] = [:]
        var projectedHistory: [String: PortfolioSnapshot] = [:]
        var currentRates: [UUID: Decimal] = [:]
        var cached = false
        let portfolios = activePortfolios.filter { selectedPortfolioIDs.contains($0.id) }
        struct HistoricalPair: Hashable {
            let from: PortfolioCurrency
            let to: PortfolioCurrency
        }
        var datesByPair: [HistoricalPair: [Date]] = [:]
        for transaction in snapshot.transactions where selectedPortfolioIDs.contains(transaction.portfolioID) {
            if transaction.price != nil, transaction.priceCurrency != reportingCurrency {
                datesByPair[HistoricalPair(from: transaction.priceCurrency, to: reportingCurrency), default: []]
                    .append(transaction.timestamp)
            }
            if transaction.fee != 0, transaction.feeCurrency != reportingCurrency {
                datesByPair[HistoricalPair(from: transaction.feeCurrency, to: reportingCurrency), default: []]
                    .append(transaction.timestamp)
            }
        }
        for point in snapshot.historicalSnapshots where selectedPortfolioIDs.contains(point.portfolioID) {
            guard let portfolio = portfolios.first(where: { $0.id == point.portfolioID }) else { continue }
            if portfolio.baseCurrency != reportingCurrency {
                datesByPair[HistoricalPair(from: portfolio.baseCurrency, to: reportingCurrency), default: []]
                    .append(point.timestamp)
            }
        }
        var historicalConversions: [HistoricalPair: [Date: Result<FXRateService.Conversion, Error>]] = [:]
        for (pair, dates) in datesByPair {
            historicalConversions[pair] = await fxService.conversions(from: pair.from, to: pair.to, on: dates)
        }
        let transactionCount = snapshot.transactions.filter { selectedPortfolioIDs.contains($0.portfolioID) }.count
        let historyCount = snapshot.historicalSnapshots.filter { selectedPortfolioIDs.contains($0.portfolioID) }.count
        let assetCount = portfolios.reduce(0) { result, portfolio in
            result
                + PortfolioAccountingEngine.uniqueAssets(
                    in: snapshot.transactions.filter { $0.portfolioID == portfolio.id }
                ).count
        }
        let operationCount = max(1, portfolios.count + transactionCount + historyCount + assetCount)
        var completedOperations = 0
        func reportProgress() {
            completedOperations += 1
            reportingConversionProgress = min(1, Double(completedOperations) / Double(operationCount))
        }
        for portfolio in portfolios {
            let baseToReport =
                portfolio.baseCurrency == reportingCurrency
                ? FXRateService.Conversion(rate: 1, observationDate: Date(), isCached: false)
                : try await fxService.conversion(from: portfolio.baseCurrency, to: reportingCurrency)
            currentRates[portfolio.id] = baseToReport.rate
            cached = cached || baseToReport.isCached
            reportProgress()
            for transaction in snapshot.transactions where transaction.portfolioID == portfolio.id {
                let pair = HistoricalPair(from: transaction.priceCurrency, to: reportingCurrency)
                let priceConversion: FXRateService.Conversion
                if transaction.price == nil || transaction.priceCurrency == reportingCurrency {
                    priceConversion = .init(rate: 1, observationDate: transaction.timestamp, isCached: false)
                } else {
                    guard let result = historicalConversions[pair]?[transaction.timestamp] else {
                        throw FXRateService.ServiceError.rateUnavailable(
                            transaction.priceCurrency, reportingCurrency, transaction.timestamp)
                    }
                    priceConversion = try result.get()
                }
                let feePair = HistoricalPair(from: transaction.feeCurrency, to: reportingCurrency)
                let feeConversion: FXRateService.Conversion
                if transaction.fee == 0 || transaction.feeCurrency == reportingCurrency {
                    feeConversion = .init(rate: 1, observationDate: transaction.timestamp, isCached: false)
                } else {
                    guard let feeResult = historicalConversions[feePair]?[transaction.timestamp] else {
                        throw FXRateService.ServiceError.rateUnavailable(
                            transaction.feeCurrency, reportingCurrency, transaction.timestamp)
                    }
                    feeConversion = try feeResult.get()
                }
                var copy = transaction
                copy.price = transaction.price.map { $0 * priceConversion.rate }
                copy.fee = transaction.fee * feeConversion.rate
                copy.priceCurrency = reportingCurrency
                copy.feeCurrency = reportingCurrency
                projectedTransactions[copy.id] = copy
                cached = cached || priceConversion.isCached || feeConversion.isCached
                reportProgress()
            }
            for asset in PortfolioAccountingEngine.uniqueAssets(
                in: snapshot.transactions.filter { $0.portfolioID == portfolio.id })
            {
                guard let quote = quotes[asset.key] else { continue }
                let conversion =
                    asset.quoteCurrency == reportingCurrency
                    ? FXRateService.Conversion(rate: 1, observationDate: quote.timestamp, isCached: false)
                    : try await fxService.conversion(from: asset.quoteCurrency, to: reportingCurrency)
                projectedQuotes[portfolio.id, default: [:]][asset.key] = PortfolioQuote(
                    price: quote.price * conversion.rate,
                    previousDayPrice: quote.previousDayPrice.map { $0 * conversion.rate }, timestamp: quote.timestamp)
                cached = cached || conversion.isCached
                reportProgress()
            }
            for point in snapshot.historicalSnapshots where point.portfolioID == portfolio.id {
                let pair = HistoricalPair(from: portfolio.baseCurrency, to: reportingCurrency)
                let conversion: FXRateService.Conversion
                if portfolio.baseCurrency == reportingCurrency {
                    conversion = .init(rate: 1, observationDate: point.timestamp, isCached: false)
                } else {
                    guard let result = historicalConversions[pair]?[point.timestamp] else {
                        throw FXRateService.ServiceError.rateUnavailable(
                            portfolio.baseCurrency, reportingCurrency, point.timestamp)
                    }
                    conversion = try result.get()
                }
                projectedHistory[point.id] = PortfolioSnapshot(
                    portfolioID: point.portfolioID, timestamp: point.timestamp,
                    value: point.value * conversion.rate,
                    netContributions: point.netContributions * conversion.rate,
                    realizedPnL: point.realizedPnL * conversion.rate,
                    unrealizedPnL: point.unrealizedPnL * conversion.rate,
                    isComplete: point.isComplete)
                cached = cached || conversion.isCached
                reportProgress()
            }
        }
        return Projection(
            transactions: projectedTransactions, quotes: projectedQuotes, history: projectedHistory,
            rates: currentRates, usesCache: cached)
    }

    private nonisolated static func preferenceKey(_ id: UUID?) -> String { id?.uuidString ?? "all-portfolios" }

    private nonisolated static func preferredCurrency(
        selectionID: UUID?, snapshot: PortfolioLedgerSnapshot, preferences: ReportingPreferences
    ) -> PortfolioCurrency {
        if let saved = preferences.currencies[preferenceKey(selectionID)] { return saved }
        guard let selectionID else { return .USD }
        return snapshot.portfolios.first { $0.id == selectionID }?.baseCurrency ?? .USD
    }

    private nonisolated static func nativeCurrency(
        selectionID: UUID?, snapshot: PortfolioLedgerSnapshot
    ) -> PortfolioCurrency {
        guard let selectionID else { return .USD }
        return snapshot.portfolios.first { $0.id == selectionID }?.baseCurrency ?? .USD
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
        guard !portfolios.isEmpty else { return [] }
        let transactions = snapshot.transactions.filter { portfolioIDs.contains($0.portfolioID) }
        return PortfolioAccountingEngine.uniqueAssets(in: transactions)
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
            await reloadReportingProjection()
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
