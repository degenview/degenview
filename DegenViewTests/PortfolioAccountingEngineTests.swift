import XCTest
@testable import DegenView

final class PortfolioAccountingEngineTests: XCTestCase {
    private let p1 = UUID(), p2 = UUID()
    private let btc = PortfolioAsset(key: "Binance:BTCUSDT", symbol: "BTC", name: "Bitcoin", source: .binance)
    private let eth = PortfolioAsset(key: "Binance:ETHUSDT", symbol: "ETH", name: "Ethereum", source: .binance)
    private let jan1 = Date(timeIntervalSince1970: 1_735_689_600)

    private func tx(_ portfolio: UUID, _ asset: PortfolioAsset, _ type: PortfolioTransactionType,
                    _ quantity: Decimal, _ price: Decimal? = nil, fee: Decimal = 0,
                    day: TimeInterval = 0, externalID: String? = nil) -> PortfolioTransaction {
        .init(portfolioID: portfolio, asset: asset, type: type, quantity: quantity, price: price,
              fee: fee, timestamp: jan1.addingTimeInterval(day * 86_400), source: externalID == nil ? .manual : .csv,
              externalTransactionID: externalID, createdAt: jan1.addingTimeInterval(day * 86_400))
    }

    func testConcreteWeightedAverageAndValuation() throws {
        let transactions = [tx(p1, btc, .buy, 1, 30_000), tx(p1, btc, .buy, 2, 45_000, day: 1)]
        let holdings = try PortfolioAccountingEngine.holdings(transactions: transactions, portfolioIDs: [p1],
            quotes: [btc.key: .init(price: 50_000, previousDayPrice: 49_000, timestamp: Date())])
        let value = try XCTUnwrap(holdings.first)
        XCTAssertEqual(value.quantity, 3); XCTAssertEqual(value.costBasis, 120_000); XCTAssertEqual(value.averageCost, 40_000)
        XCTAssertEqual(value.currentValue, 150_000); XCTAssertEqual(value.unrealizedPnL, 30_000); XCTAssertEqual(value.pnlPercent, Decimal(string: "0.25"))
    }

    func testPortfolioQuoteDictionaryCodableRoundTrip() throws {
        let timestamp = Date(timeIntervalSince1970: 1_735_700_123)
        let values = [btc.key: PortfolioQuote(price: Decimal(string: "98765.4321")!,
            previousDayPrice: Decimal(string: "95432.10"), timestamp: timestamp)]
        let decoded = try JSONDecoder().decode([String: PortfolioQuote].self,
            from: JSONEncoder().encode(values))
        XCTAssertEqual(decoded, values)
        XCTAssertEqual(decoded[btc.key]?.timestamp, timestamp)
    }

    func testInitialQuoteLoadingDecisions() {
        let usd = Portfolio(id: p1, name: "USD", baseCurrency: .USD)
        let eur = Portfolio(id: p2, name: "EUR", baseCurrency: .EUR)
        let usdTransaction = tx(p1, btc, .buy, 1, 10)
        let staleQuote = PortfolioQuote(price: 50_000,
            timestamp: Date(timeIntervalSinceNow: -3_600))

        XCTAssertFalse(PortfolioStore.needsInitialQuoteLoad(snapshot: .empty, quotes: [:]))

        let usdSnapshot = PortfolioLedgerSnapshot(portfolios: [usd],
            transactions: [usdTransaction], selectedPortfolioID: p1)
        XCTAssertTrue(PortfolioStore.needsInitialQuoteLoad(snapshot: usdSnapshot, quotes: [:]))
        XCTAssertFalse(PortfolioStore.needsInitialQuoteLoad(snapshot: usdSnapshot,
            quotes: [btc.key: staleQuote]))

        let partialSnapshot = PortfolioLedgerSnapshot(portfolios: [usd],
            transactions: [usdTransaction, tx(p1, eth, .buy, 1, 10)], selectedPortfolioID: p1)
        XCTAssertTrue(PortfolioStore.needsInitialQuoteLoad(snapshot: partialSnapshot,
            quotes: [btc.key: staleQuote]))

        var unsupportedAsset = btc
        unsupportedAsset = PortfolioAsset(key: unsupportedAsset.key, symbol: unsupportedAsset.symbol,
            name: unsupportedAsset.name, source: unsupportedAsset.source, quoteCurrency: .EUR)
        let unsupportedQuoteSnapshot = PortfolioLedgerSnapshot(portfolios: [usd],
            transactions: [tx(p1, unsupportedAsset, .buy, 1, 10)], selectedPortfolioID: p1)
        XCTAssertFalse(PortfolioStore.needsInitialQuoteLoad(snapshot: unsupportedQuoteSnapshot, quotes: [:]))

        let unsupportedDashboard = PortfolioLedgerSnapshot(portfolios: [eur],
            transactions: [tx(p2, btc, .buy, 1, 10)], selectedPortfolioID: p2)
        XCTAssertFalse(PortfolioStore.needsInitialQuoteLoad(snapshot: unsupportedDashboard, quotes: [:]))
    }

    func testPartialSellRealizedAndRemainingPnL() throws {
        let transactions = [tx(p1, btc, .buy, 1, 30_000), tx(p1, btc, .buy, 2, 45_000, day: 1), tx(p1, btc, .sell, 1, 55_000, day: 2)]
        let h = try XCTUnwrap(PortfolioAccountingEngine.holdings(transactions: transactions, portfolioIDs: [p1], quotes: [btc.key: .init(price: 50_000, timestamp: Date())]).first)
        XCTAssertEqual(h.quantity, 2); XCTAssertEqual(h.averageCost, 40_000); XCTAssertEqual(h.realizedPnL, 15_000)
        XCTAssertEqual(h.currentValue, 100_000); XCTAssertEqual(h.unrealizedPnL, 20_000)
    }

    func testFeesIncreaseBuyBasisAndReduceSellProceeds() throws {
        let transactions = [tx(p1, btc, .buy, 2, 100, fee: 10), tx(p1, btc, .sell, 1, 150, fee: 5, day: 1)]
        let h = try XCTUnwrap(PortfolioAccountingEngine.holdings(transactions: transactions, portfolioIDs: [p1]).first)
        XCTAssertEqual(h.quantity, 1); XCTAssertEqual(h.averageCost, 105); XCTAssertEqual(h.realizedPnL, 40)
    }

    func testTransferOutIsNotRealizedSale() throws {
        let transactions = [tx(p1, btc, .transferIn, 2, 100), tx(p1, btc, .transferOut, 1, nil, fee: 2, day: 1)]
        let h = try XCTUnwrap(PortfolioAccountingEngine.holdings(transactions: transactions, portfolioIDs: [p1]).first)
        XCTAssertEqual(h.quantity, 1); XCTAssertEqual(h.costBasis, 102); XCTAssertEqual(h.realizedPnL, 0)
    }

    func testHistoricalNegativeHoldingsRejected() {
        let transactions = [tx(p1, btc, .sell, 1, 100), tx(p1, btc, .buy, 1, 90, day: 1)]
        XCTAssertThrowsError(try PortfolioAccountingEngine.holdings(transactions: transactions, portfolioIDs: [p1]))
    }

    func testHistoricalHoldingsAreTransactionAware() throws {
        let transactions = [tx(p1, btc, .buy, 1, 30_000), tx(p1, eth, .buy, 1, 2_000, day: 31)]
        let january = try PortfolioAccountingEngine.holdings(transactions: transactions, portfolioIDs: [p1], through: jan1.addingTimeInterval(14 * 86_400))
        let february = try PortfolioAccountingEngine.holdings(transactions: transactions, portfolioIDs: [p1], through: jan1.addingTimeInterval(45 * 86_400))
        XCTAssertEqual(january.map(\.asset.key), [btc.key]); XCTAssertEqual(Set(february.map(\.asset.key)), [btc.key, eth.key])
    }

    func testMultiplePortfoliosStayIndependentAndAggregate() throws {
        let values = [tx(p1, btc, .buy, 1, 10), tx(p2, btc, .buy, 2, 10), tx(p2, eth, .buy, 5, 10)]
        XCTAssertEqual(try PortfolioAccountingEngine.holdings(transactions: values, portfolioIDs: [p1]).first?.quantity, 1)
        let all = try PortfolioAccountingEngine.holdings(transactions: values, portfolioIDs: [p1, p2])
        XCTAssertEqual(all.first { $0.asset.key == btc.key }?.quantity, 3); XCTAssertEqual(all.first { $0.asset.key == eth.key }?.quantity, 5)
    }

    func testAllocationUsesCurrentValues() throws {
        let values = [tx(p1, btc, .buy, 1, 10), tx(p1, eth, .buy, 2, 10)]
        let holdings = try PortfolioAccountingEngine.holdings(transactions: values, portfolioIDs: [p1], quotes: [btc.key: .init(price: 80, timestamp: Date()), eth.key: .init(price: 10, timestamp: Date())])
        XCTAssertEqual(holdings.first { $0.asset.key == btc.key }?.allocation, Decimal(string: "0.8"))
    }

    func testEditInvalidatesOnlyAffectedSnapshotSuffix() async throws {
        let ledger = PortfolioLedger(now: { self.jan1 })
        let id = try await ledger.createPortfolio(name: "Long-Term", currency: .USD)
        var original = tx(id, btc, .buy, 1, 30_000)
        try await ledger.add(original)
        let before = PortfolioSnapshot(portfolioID: id, timestamp: jan1.addingTimeInterval(-86_400), value: 0, netContributions: 0, realizedPnL: 0, unrealizedPnL: 0, isComplete: true)
        let after = PortfolioSnapshot(portfolioID: id, timestamp: jan1.addingTimeInterval(86_400), value: 30_000, netContributions: 30_000, realizedPnL: 0, unrealizedPnL: 0, isComplete: true)
        try await ledger.storeSnapshots([before, after], for: id, from: .distantPast)
        original.quantity = 2; try await ledger.update(original)
        let snapshot = await ledger.snapshot()
        XCTAssertTrue(snapshot.historicalSnapshots.contains(before)); XCTAssertFalse(snapshot.historicalSnapshots.contains(after)); XCTAssertEqual(snapshot.invalidatedAfter[id], jan1)
        XCTAssertEqual(try PortfolioAccountingEngine.holdings(transactions: snapshot.transactions, portfolioIDs: [id]).first?.quantity, 2)
    }

    func testCompletePortfolioHistoryNeedsNoRebuild() {
        let today = jan1.addingTimeInterval(4 * 86_400)
        let state = PortfolioLedgerSnapshot(
            historicalSnapshots: [snapshotPoint(portfolio: p1, day: 4)])
        XCTAssertNil(PortfolioStore.historyRebuildStart(for: p1, in: state, today: today))
    }

    func testMissingPortfolioHistoryAppendsFromNextDay() {
        let calendar = Calendar(identifier: .gregorian)
        let state = PortfolioLedgerSnapshot(
            historicalSnapshots: [snapshotPoint(portfolio: p1, day: 2)])
        XCTAssertEqual(
            PortfolioStore.historyRebuildStart(for: p1, in: state, today: jan1.addingTimeInterval(4 * 86_400)),
            calendar.date(byAdding: .day, value: 1,
                          to: calendar.startOfDay(for: jan1.addingTimeInterval(2 * 86_400)))
        )
    }

    func testInvalidatedPortfolioHistoryRebuildsAffectedSuffix() {
        let calendar = Calendar(identifier: .gregorian)
        let invalidation = jan1.addingTimeInterval(2 * 86_400 + 3_600)
        let state = PortfolioLedgerSnapshot(
            historicalSnapshots: [snapshotPoint(portfolio: p1, day: 1)],
            invalidatedAfter: [p1: invalidation])
        XCTAssertEqual(
            PortfolioStore.historyRebuildStart(for: p1, in: state, today: jan1.addingTimeInterval(4 * 86_400)),
            calendar.startOfDay(for: invalidation)
        )
    }

    func testAggregateHistoryHasStableIdentityAndTotals() throws {
        let first = snapshotPoint(portfolio: p1, day: 1)
        let second = PortfolioSnapshot(portfolioID: p2, timestamp: first.timestamp,
            value: 2, netContributions: 2, realizedPnL: 1, unrealizedPnL: 1, isComplete: false)
        let aggregate1 = try XCTUnwrap(PortfolioStore.aggregateHistory([first, second]).first)
        let aggregate2 = try XCTUnwrap(PortfolioStore.aggregateHistory([first, second]).first)
        XCTAssertEqual(aggregate1.id, aggregate2.id)
        XCTAssertEqual(aggregate1.value, 3)
        XCTAssertEqual(aggregate1.netContributions, 3)
        XCTAssertFalse(aggregate1.isComplete)
    }

    private func snapshotPoint(portfolio: UUID, day: TimeInterval) -> PortfolioSnapshot {
        PortfolioSnapshot(portfolioID: portfolio, timestamp: jan1.addingTimeInterval(day * 86_400),
            value: 1, netContributions: 1, realizedPnL: 0, unrealizedPnL: 0, isComplete: true)
    }

    func testPortfolioCreationDuplicationDeletionAndPersistenceCodable() async throws {
        let ledger = PortfolioLedger(now: { self.jan1 })
        let a = try await ledger.createPortfolio(name: "A", currency: .USD); try await ledger.add(tx(a, btc, .buy, 1, 10))
        let b = try await ledger.duplicatePortfolio(a); var state = await ledger.snapshot()
        XCTAssertEqual(state.portfolios.count, 2); XCTAssertEqual(state.transactions.first { $0.portfolioID == b }?.quantity, 1)
        try await ledger.deletePortfolio(a); state = await ledger.snapshot(); XCTAssertEqual(state.portfolios.map(\.id), [b]); XCTAssertFalse(state.transactions.contains { $0.portfolioID == a })
        XCTAssertNoThrow(try JSONDecoder().decode(PortfolioLedgerSnapshot.self, from: JSONEncoder().encode(state)))
    }

    func testCSVPreviewIsAtomicAndDeduplicatesExternalIDs() async throws {
        let portfolio = Portfolio(id: p1, name: "Main")
        let csv = "\(PortfolioCSVService.header)\nMain,Binance:BTCUSDT,BTC,Bitcoin,Binance,Buy,0.5,42000,USD,10,USD,2025-01-03T14:21:00Z,First purchase,abc-1"
        let preview = PortfolioCSVService.preview(csv, portfolios: [portfolio]); XCTAssertTrue(preview.isValid); XCTAssertEqual(preview.transactions.count, 1)
        let ledger = PortfolioLedger(snapshot: .init(portfolios: [portfolio]))
        try await ledger.importTransactions(preview.transactions)
        do { try await ledger.importTransactions(preview.transactions); XCTFail("Expected duplicate") } catch { XCTAssertEqual(error as? PortfolioError, .duplicateExternalTransaction) }
        let imported = await ledger.snapshot()
        XCTAssertEqual(imported.transactions.count, 1)
    }

    func testPrivacyNeverReturnsSensitiveVisibleOrAccessibilityText() {
        XCTAssertEqual(PortfolioPrivacy.sensitive("$128,430", enabled: true), "••••••••")
        XCTAssertEqual(PortfolioPrivacy.accessibility("Portfolio balance, $128,430", enabled: true, hiddenDescription: "Portfolio balance hidden"), "Portfolio balance hidden")
        XCTAssertFalse(PortfolioPrivacy.sensitive("secret", enabled: true).contains("secret"))
    }

    func testPortfolioValueChangeCalculatesAmountPercentageAndDirection() {
        let gain = PortfolioValueChange(from: 100, to: 125)
        XCTAssertEqual(gain.amount, 25)
        XCTAssertEqual(gain.percentage, Decimal(string: "0.25"))
        XCTAssertEqual(gain.direction, .up)

        let loss = PortfolioValueChange(from: 200, to: 150)
        XCTAssertEqual(loss.amount, -50)
        XCTAssertEqual(loss.percentage, Decimal(string: "-0.25"))
        XCTAssertEqual(loss.direction, .down)

        let unchanged = PortfolioValueChange(from: 75, to: 75)
        XCTAssertEqual(unchanged.amount, 0)
        XCTAssertEqual(unchanged.percentage, 0)
        XCTAssertEqual(unchanged.direction, .unchanged)
    }

    func testPortfolioValueChangeOmitsPercentageForZeroBaseline() {
        let change = PortfolioValueChange(from: 0, to: 100)
        XCTAssertEqual(change.amount, 100)
        XCTAssertNil(change.percentage)
        XCTAssertEqual(change.direction, .up)
    }

    func testCoinMarketCapImportParsesTimezoneQuotedThousandsAndMissingFees() throws {
        let csv = """
        Date (UTC+2:00),Token,Type,Price (USD),Amount,Total value (USD),Fee,Fee Currency,Notes
        "2026-08-20 09:25:00","BTC","buy","69,608.61","0.001328","92.47","4.0000","USD","DCA"
        "2025-10-02 23:00:00","FLOKI","buy","0.00008785","1,012,712.00","88.97","--","",""
        """
        let preview = PortfolioCSVService.previewCoinMarketCap(csv)
        XCTAssertTrue(preview.isValid); XCTAssertEqual(preview.rows.count, 2)
        XCTAssertEqual(preview.rows[0].price, Decimal(string: "69608.61")); XCTAssertEqual(preview.rows[0].amount, Decimal(string: "0.001328"))
        XCTAssertEqual(preview.rows[1].amount, Decimal(string: "1012712.00")); XCTAssertEqual(preview.rows[1].fee, 0)
        XCTAssertEqual(ISO8601DateFormatter().string(from: preview.rows[0].timestamp), "2026-08-20T07:25:00Z")

        let transactions = preview.transactions(portfolioID: p1, mappings: ["BTC": btc, "FLOKI": .init(key: "CoinGecko:floki", symbol: "FLOKI", name: "FLOKI", source: .coingecko)])
        XCTAssertTrue(transactions.isValid); XCTAssertEqual(transactions.transactions.count, 2)
        XCTAssertEqual(transactions.transactions.first?.source, .coinMarketCap)
        XCTAssertTrue(transactions.transactions.allSatisfy { $0.externalTransactionID?.hasPrefix("cmc:") == true })
    }

    func testCoinMarketCapImportSkipsUnmappedTickersAndRejectsUnconvertedForeignFee() {
        let csv = """
        Date (UTC+2:00),Token,Type,Price (USD),Amount,Total value (USD),Fee,Fee Currency,Notes
        "2024-12-12 11:00:00","FLOKI","buy","0.0002876","326,407.00","93.89","3.0000","EUR",""
        """
        let preview = PortfolioCSVService.previewCoinMarketCap(csv)
        let unmapped = preview.transactions(portfolioID: p1, mappings: [:])
        XCTAssertTrue(unmapped.isValid); XCTAssertTrue(unmapped.transactions.isEmpty)
        XCTAssertTrue(unmapped.warnings.contains { $0.contains("Only mapped tickers will be imported") })
        let asset = PortfolioAsset(key: "CoinGecko:floki", symbol: "FLOKI", name: "FLOKI", source: .coingecko)
        let mapped = preview.transactions(portfolioID: p1, mappings: ["FLOKI": asset])
        XCTAssertFalse(mapped.isValid); XCTAssertTrue(mapped.errors.contains { $0.contains("historical") })
        let rowID = try! XCTUnwrap(preview.rows.first?.id)
        let converted = preview.transactions(portfolioID: p1, mappings: ["FLOKI": asset], feeFXRates: [rowID: Decimal(string: "1.08")!])
        XCTAssertTrue(converted.isValid); XCTAssertEqual(converted.transactions.first?.fee, Decimal(string: "3.24"))
        XCTAssertTrue(converted.transactions.first?.notes.contains("CMC fee") == true)
        let skipped = preview.transactions(portfolioID: p1, mappings: ["FLOKI": asset], skippedRowIDs: [rowID])
        XCTAssertTrue(skipped.isValid); XCTAssertTrue(skipped.transactions.isEmpty)
    }

    func testAutoMapperPrefersRequestedBinanceQuotePair() {
        let results = [
            TickerSearchResult(symbol: "FLOKI/TRY", fullSymbol: "FLOKITRY", source: .binance, price: nil),
            TickerSearchResult(symbol: "FLOKI/USDT", fullSymbol: "FLOKIUSDT", source: .binance, price: nil),
            TickerSearchResult(symbol: "FLOKI/EUR", fullSymbol: "FLOKIEUR", source: .binance, price: nil)
        ]
        XCTAssertEqual(PortfolioAssetAutoMapper.bestPair(in: results, token: "FLOKI", preferredQuotes: ["USDT", "USD"])?.fullSymbol, "FLOKIUSDT")
        XCTAssertEqual(PortfolioAssetAutoMapper.bestPair(in: results, token: "FLOKI", preferredQuotes: ["EUR", "USDT"])?.fullSymbol, "FLOKIEUR")
    }

    func testImportValidatesNewestFirstExportInChronologicalOrder() async throws {
        let portfolio = Portfolio(id: p1, name: "CMC")
        let olderBuy = tx(p1, eth, .buy, 2, 2_000, day: 0)
        let laterSell = tx(p1, eth, .sell, 1, 3_000, day: 30)
        let ledger = PortfolioLedger(snapshot: .init(portfolios: [portfolio]))

        // CoinMarketCap exports this exact reverse-chronological shape.
        try await ledger.importTransactions([laterSell, olderBuy])

        let snapshot = await ledger.snapshot()
        let holding = try XCTUnwrap(PortfolioAccountingEngine.holdings(
            transactions: snapshot.transactions, portfolioIDs: [p1]
        ).first)
        XCTAssertEqual(holding.quantity, 1)
        XCTAssertEqual(holding.realizedPnL, 1_000)
    }

    func testRepeatedTransactionsProduceOneUniqueAssetWithoutDictionaryTrap() {
        let transactions = [
            tx(p1, btc, .buy, 1, 30_000),
            tx(p1, btc, .buy, 2, 40_000, day: 1),
            tx(p1, btc, .sell, 1, 50_000, day: 2),
            tx(p1, eth, .buy, 3, 2_000, day: 3)
        ]
        let assets = PortfolioAccountingEngine.uniqueAssets(in: transactions)
        XCTAssertEqual(assets.count, 2)
        XCTAssertEqual(Set(assets.map(\.key)), [btc.key, eth.key])
    }

    func testAssetRemapIsScopedAtomicAndRecalculatesLedger() async throws {
        let otherBTC = PortfolioAsset(key: "CoinGecko:bitcoin", symbol: "BTC", name: "Bitcoin", source: .coingecko)
        let portfolios = [Portfolio(id: p1, name: "Main"), Portfolio(id: p2, name: "Trading")]
        let original = [
            tx(p1, btc, .buy, 2, 30_000), tx(p1, btc, .sell, 1, 40_000, day: 1),
            tx(p2, btc, .buy, 3, 35_000)
        ]
        let ledger = PortfolioLedger(snapshot: .init(portfolios: portfolios, transactions: original))
        try await ledger.remapAsset(from: btc.key, to: otherBTC, portfolioIDs: [p1])
        let state = await ledger.snapshot()

        XCTAssertTrue(state.transactions.filter { $0.portfolioID == p1 }.allSatisfy { $0.asset.key == otherBTC.key })
        XCTAssertTrue(state.transactions.filter { $0.portfolioID == p2 }.allSatisfy { $0.asset.key == btc.key })
        let holding = try XCTUnwrap(PortfolioAccountingEngine.holdings(transactions: state.transactions, portfolioIDs: [p1]).first)
        XCTAssertEqual(holding.quantity, 1); XCTAssertEqual(holding.realizedPnL, 10_000)
        XCTAssertEqual(state.invalidatedAfter[p1], jan1)
    }
}
