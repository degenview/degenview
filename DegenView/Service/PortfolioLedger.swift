import Foundation

actor PortfolioLedger {
    typealias Persistence = @Sendable (PortfolioLedgerSnapshot) async throws -> Void
    private var state: PortfolioLedgerSnapshot
    private let persist: Persistence?
    private let now: @Sendable () -> Date

    init(
        snapshot: PortfolioLedgerSnapshot = .empty, now: @escaping @Sendable () -> Date = { Date() },
        persist: Persistence? = nil
    ) {
        state = snapshot
        self.now = now
        self.persist = persist
    }

    func snapshot() -> PortfolioLedgerSnapshot { state }

    @discardableResult
    func createPortfolio(name: String, currency: PortfolioCurrency) async throws -> UUID {
        let portfolio = Portfolio(
            name: name.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty ?? "Unnamed Portfolio",
            baseCurrency: currency, createdAt: now(), updatedAt: now())
        state.portfolios.append(portfolio)
        state.selectedPortfolioID = portfolio.id
        try await save()
        return portfolio.id
    }

    func select(_ id: UUID?) async throws {
        if let id, !state.portfolios.contains(where: { $0.id == id }) { throw PortfolioError.portfolioNotFound }
        state.selectedPortfolioID = id
        try await save()
    }

    func updatePortfolio(_ portfolio: Portfolio) async throws {
        guard let index = state.portfolios.firstIndex(where: { $0.id == portfolio.id }) else {
            throw PortfolioError.portfolioNotFound
        }
        var value = portfolio
        value.updatedAt = now()
        state.portfolios[index] = value
        try await save()
    }

    @discardableResult
    func duplicatePortfolio(_ id: UUID) async throws -> UUID {
        guard let original = state.portfolios.first(where: { $0.id == id }) else {
            throw PortfolioError.portfolioNotFound
        }
        let copy = Portfolio(
            name: "\(original.name) Copy", baseCurrency: original.baseCurrency, createdAt: now(), updatedAt: now())
        state.portfolios.append(copy)
        state.transactions += state.transactions.filter { $0.portfolioID == id }.map {
            PortfolioTransaction(
                portfolioID: copy.id, asset: $0.asset, type: $0.type, quantity: $0.quantity,
                price: $0.price, priceCurrency: $0.priceCurrency, fee: $0.fee, feeCurrency: $0.feeCurrency,
                timestamp: $0.timestamp, notes: $0.notes, source: .manual)
        }
        state.selectedPortfolioID = copy.id
        invalidate(
            copy.id, from: state.transactions.filter { $0.portfolioID == copy.id }.map(\.timestamp).min() ?? now())
        try await save()
        return copy.id
    }

    func reorder(from offsets: IndexSet, to destination: Int) async throws {
        let moving = offsets.map { state.portfolios[$0] }
        for index in offsets.sorted(by: >) { state.portfolios.remove(at: index) }
        let removedBefore = offsets.filter { $0 < destination }.count
        state.portfolios.insert(contentsOf: moving, at: max(0, destination - removedBefore))
        try await save()
    }

    func deletePortfolio(_ id: UUID) async throws {
        guard state.portfolios.contains(where: { $0.id == id }) else { throw PortfolioError.portfolioNotFound }
        state.portfolios.removeAll { $0.id == id }
        state.transactions.removeAll { $0.portfolioID == id }
        state.historicalSnapshots.removeAll { $0.portfolioID == id }
        state.invalidatedAfter[id] = nil
        if state.selectedPortfolioID == id { state.selectedPortfolioID = state.portfolios.first?.id }
        try await save()
    }

    func add(_ transaction: PortfolioTransaction) async throws {
        guard state.portfolios.contains(where: { $0.id == transaction.portfolioID }) else {
            throw PortfolioError.portfolioNotFound
        }
        try PortfolioAccountingEngine.validate(transaction, in: state.transactions)
        state.transactions.append(transaction)
        touch(transaction.portfolioID)
        invalidate(transaction.portfolioID, from: transaction.timestamp)
        try await save()
    }

    func update(_ transaction: PortfolioTransaction) async throws {
        guard let index = state.transactions.firstIndex(where: { $0.id == transaction.id }) else {
            throw PortfolioError.transactionNotFound
        }
        let old = state.transactions[index]
        try PortfolioAccountingEngine.validate(transaction, replacing: transaction.id, in: state.transactions)
        state.transactions[index] = transaction
        touch(transaction.portfolioID)
        invalidate(old.portfolioID, from: min(old.timestamp, transaction.timestamp))
        if old.portfolioID != transaction.portfolioID {
            invalidate(transaction.portfolioID, from: transaction.timestamp)
        }
        try await save()
    }

    func deleteTransaction(_ id: UUID) async throws {
        guard let transaction = state.transactions.first(where: { $0.id == id }) else {
            throw PortfolioError.transactionNotFound
        }
        let remaining = state.transactions.filter { $0.id != id }
        _ = try PortfolioAccountingEngine.holdings(transactions: remaining, portfolioIDs: [transaction.portfolioID])
        state.transactions = remaining
        touch(transaction.portfolioID)
        invalidate(transaction.portfolioID, from: transaction.timestamp)
        try await save()
    }

    /// Rebind every ledger event for an asset within the requested portfolios.
    /// This changes canonical identity, so the complete prospective ledger is
    /// validated before any state is committed.
    func remapAsset(
        from sourceKey: String, to destination: PortfolioAsset,
        portfolioIDs: Set<UUID>
    ) async throws {
        guard sourceKey != destination.key else { return }
        var candidate = state.transactions
        var affectedStarts: [UUID: Date] = [:]
        var didChange = false

        for index in candidate.indices
        where portfolioIDs.contains(candidate[index].portfolioID) && candidate[index].asset.key == sourceKey {
            let portfolioID = candidate[index].portfolioID
            affectedStarts[portfolioID] = min(
                affectedStarts[portfolioID] ?? candidate[index].timestamp,
                candidate[index].timestamp)
            candidate[index].asset = destination
            candidate[index].updatedAt = now()
            didChange = true
        }
        guard didChange else { return }

        for portfolioID in affectedStarts.keys {
            _ = try PortfolioAccountingEngine.holdings(
                transactions: candidate, portfolioIDs: [portfolioID]
            )
        }

        state.transactions = candidate
        for (portfolioID, start) in affectedStarts {
            touch(portfolioID)
            invalidate(portfolioID, from: start)
        }
        try await save()
    }

    /// Import is all-or-nothing: validate the complete candidate ledger before committing.
    func importTransactions(_ values: [PortfolioTransaction]) async throws {
        var candidate = state.transactions
        // CoinMarketCap and many exchanges export newest-first. Validate additions
        // chronologically so an older acquisition is present before a later sale,
        // regardless of row order in the source file.
        let chronological = values.sorted {
            $0.timestamp == $1.timestamp ? $0.createdAt < $1.createdAt : $0.timestamp < $1.timestamp
        }
        for value in chronological {
            try PortfolioAccountingEngine.validate(value, in: candidate)
            candidate.append(value)
        }
        state.transactions = candidate
        for value in values {
            touch(value.portfolioID)
            invalidate(value.portfolioID, from: value.timestamp)
        }
        try await save()
    }

    func storeSnapshots(_ snapshots: [PortfolioSnapshot], for portfolioID: UUID, from start: Date) async throws {
        state.historicalSnapshots.removeAll { $0.portfolioID == portfolioID && $0.timestamp >= start }
        state.historicalSnapshots.append(contentsOf: snapshots)
        state.invalidatedAfter[portfolioID] = nil
        try await save()
    }

    private func invalidate(_ id: UUID, from date: Date) {
        state.invalidatedAfter[id] = min(state.invalidatedAfter[id] ?? date, date)
        state.historicalSnapshots.removeAll { $0.portfolioID == id && $0.timestamp >= date }
    }
    private func touch(_ id: UUID) {
        if let i = state.portfolios.firstIndex(where: { $0.id == id }) { state.portfolios[i].updatedAt = now() }
    }
    private func save() async throws { try await persist?(state) }
}

private extension String { var nilIfEmpty: String? { isEmpty ? nil : self } }
