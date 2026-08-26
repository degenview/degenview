import Foundation

/// Pure weighted-average accounting. Static accounting changes only with ledger edits;
/// live valuation is applied separately so quote ticks never replay transactions.
enum PortfolioAccountingEngine {
    private struct Position {
        var asset: PortfolioAsset
        var quantity: Decimal = 0
        var costBasis: Decimal = 0
        var realizedPnL: Decimal = 0
    }

    static func holdings(
        transactions: [PortfolioTransaction], portfolioIDs: Set<UUID>,
        through date: Date = .distantFuture,
        quotes: [String: PortfolioQuote] = [:]
    ) throws -> [PortfolioHolding] {
        var positions: [String: Position] = [:]
        for transaction
            in transactions
            .filter({ portfolioIDs.contains($0.portfolioID) && $0.timestamp <= date })
            .sorted(by: transactionOrder)
        {
            try apply(transaction, to: &positions)
        }
        var holdings = positions.values.filter { $0.quantity != 0 }.map { position -> PortfolioHolding in
            let quote = quotes[position.asset.key]
            let value = quote.map { position.quantity * $0.price }
            let unrealized = value.map { $0 - position.costBasis }
            return PortfolioHolding(
                asset: position.asset, quantity: position.quantity,
                costBasis: position.costBasis,
                averageCost: position.quantity == 0 ? 0 : position.costBasis / position.quantity,
                realizedPnL: position.realizedPnL, currentPrice: quote?.price,
                previousDayPrice: quote?.previousDayPrice, currentValue: value,
                unrealizedPnL: unrealized, allocation: 0,
                isPriceStale: quote.map { Date().timeIntervalSince($0.timestamp) > 300 } ?? true)
        }
        let total = holdings.compactMap(\.currentValue).reduce(0, +)
        if total != 0 {
            for index in holdings.indices { holdings[index].allocation = (holdings[index].currentValue ?? 0) / total }
        }
        return holdings
    }

    /// A ledger normally contains many events for the same asset. Build this
    /// lookup with an explicit merge policy rather than
    /// `Dictionary(uniqueKeysWithValues:)`, which traps on the second buy.
    static func uniqueAssets(in transactions: [PortfolioTransaction]) -> [PortfolioAsset] {
        Array(
            Dictionary(
                transactions.map { ($0.asset.key, $0.asset) },
                uniquingKeysWith: { existing, _ in existing }
            ).values)
    }

    static func validate(
        _ transaction: PortfolioTransaction, replacing id: UUID? = nil,
        in transactions: [PortfolioTransaction]
    ) throws {
        guard transaction.quantity > 0 else { throw PortfolioError.invalidQuantity }
        if [.buy, .sell].contains(transaction.type), transaction.price == nil { throw PortfolioError.missingPrice }
        guard transaction.fee >= 0 else { throw PortfolioError.invalidQuantity }
        guard transaction.priceCurrency == transaction.feeCurrency || transaction.fee == 0 else {
            throw PortfolioError.unsupportedCurrency(transaction.feeCurrency, transaction.priceCurrency)
        }
        if transaction.source != .manual, let external = transaction.externalTransactionID,
            transactions.contains(where: {
                $0.id != id && $0.portfolioID == transaction.portfolioID && $0.source == transaction.source
                    && $0.externalTransactionID == external
            })
        {
            throw PortfolioError.duplicateExternalTransaction
        }
        var prospective = transactions.filter { $0.id != id && $0.portfolioID == transaction.portfolioID }
        prospective.append(transaction)
        _ = try holdings(transactions: prospective, portfolioIDs: [transaction.portfolioID])
    }

    static func netContributions(_ transactions: [PortfolioTransaction], portfolioIDs: Set<UUID>, through date: Date)
        -> Decimal
    {
        transactions.filter { portfolioIDs.contains($0.portfolioID) && $0.timestamp <= date }.reduce(0) { result, tx in
            let gross = tx.quantity * (tx.price ?? 0)
            switch tx.type {
            case .buy: return result + gross + tx.fee
            case .sell: return result - gross + tx.fee
            case .transferIn: return result + gross + tx.fee
            case .transferOut: return result - gross + tx.fee
            default: return result + tx.fee
            }
        }
    }

    private static func apply(_ tx: PortfolioTransaction, to positions: inout [String: Position]) throws {
        var position = positions[tx.asset.key] ?? Position(asset: tx.asset)
        let price = tx.price ?? 0
        switch tx.type {
        case .buy:
            position.quantity += tx.quantity
            position.costBasis += tx.quantity * price + tx.fee
        case .transferIn, .reward, .stakingReward, .airdrop, .mining, .interest:
            position.quantity += tx.quantity
            position.costBasis += tx.quantity * price + tx.fee
        case .sell:
            let disposedBasis = try remove(tx.quantity, from: &position)
            position.realizedPnL += tx.quantity * price - tx.fee - disposedBasis
        case .transferOut:
            _ = try remove(tx.quantity, from: &position)
            position.costBasis += tx.fee
        case .fee:
            let disposedBasis = try remove(tx.quantity, from: &position)
            position.realizedPnL -= disposedBasis + tx.fee
        case .adjustment:
            if tx.quantity >= 0 {
                position.quantity += tx.quantity
                position.costBasis += tx.quantity * price + tx.fee
            }
        }
        if position.quantity == 0 { position.costBasis = 0 }
        positions[tx.asset.key] = position
    }

    private static func remove(_ quantity: Decimal, from position: inout Position) throws -> Decimal {
        guard position.quantity >= quantity else {
            throw PortfolioError.insufficientHoldings(available: position.quantity)
        }
        let average = position.quantity == 0 ? 0 : position.costBasis / position.quantity
        let removedBasis = average * quantity
        position.quantity -= quantity
        position.costBasis -= removedBasis
        if position.quantity == 0 { position.costBasis = 0 }
        return removedBasis
    }

    private static func transactionOrder(_ lhs: PortfolioTransaction, _ rhs: PortfolioTransaction) -> Bool {
        lhs.timestamp == rhs.timestamp ? lhs.createdAt < rhs.createdAt : lhs.timestamp < rhs.timestamp
    }
}
