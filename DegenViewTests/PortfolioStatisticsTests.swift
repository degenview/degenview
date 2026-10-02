import XCTest

@testable import DegenView

final class PortfolioStatisticsTests: XCTestCase {
    private let portfolioID = UUID()

    private func asset(_ ticker: String) -> PortfolioAsset {
        PortfolioAsset(key: "binance:\(ticker)USDT", symbol: "\(ticker)/USDT", name: "\(ticker)/USDT", source: .binance)
    }

    private func holding(
        _ ticker: String, quantity: Decimal = 1, cost: Decimal, value: Decimal?, realized: Decimal = 0,
        previousPrice: Decimal? = nil, allocation: Decimal = 0
    ) -> PortfolioHolding {
        PortfolioHolding(
            asset: asset(ticker), quantity: quantity, costBasis: cost, averageCost: cost / quantity,
            realizedPnL: realized, currentPrice: value.map { $0 / quantity }, previousDayPrice: previousPrice,
            currentValue: value, unrealizedPnL: value.map { $0 - cost }, allocation: allocation, isPriceStale: false)
    }

    private func snapshot(_ day: Int, value: Decimal, contributions: Decimal) -> PortfolioSnapshot {
        PortfolioSnapshot(
            portfolioID: portfolioID, timestamp: Date(timeIntervalSince1970: Double(day) * 86_400), value: value,
            netContributions: contributions, realizedPnL: 0, unrealizedPnL: value - contributions, isComplete: true)
    }

    private func transaction(
        _ type: PortfolioTransactionType, day: Int, fee: Decimal = 0
    ) -> PortfolioTransaction {
        PortfolioTransaction(
            portfolioID: portfolioID, asset: asset("BTC"), type: type, quantity: 1, price: 100, fee: fee,
            timestamp: Date(timeIntervalSince1970: Double(day) * 86_400))
    }

    // MARK: - Statistics

    func testTotalsSumPricedHoldingsAndSkipUnpricedOnes() {
        let stats = PortfolioStatistics(holdings: [
            holding("BTC", cost: 100, value: 150, realized: 10),
            holding("ETH", cost: 200, value: 180, realized: -5),
            holding("XYZ", cost: 50, value: nil),
        ])

        XCTAssertEqual(stats.totalValue, 330)
        XCTAssertEqual(stats.costBasis, 350)
        XCTAssertEqual(stats.unrealizedPnL, 30)
        XCTAssertEqual(stats.realizedPnL, 5)
        XCTAssertEqual(stats.totalPnL, 35)
        // Unrealized % is against the cost of the assets that have a price: 30 / 300.
        XCTAssertEqual(stats.unrealizedPercent, Decimal(string: "0.1"))
        XCTAssertEqual(stats.assetResults.count, 2)
    }

    func testBestAndWorstFollowReturnPercentNotAmount() {
        // BTC gains more money (+50) but ETH returns more (+100%).
        let stats = PortfolioStatistics(holdings: [
            holding("BTC", cost: 1000, value: 1050),
            holding("ETH", cost: 10, value: 20),
            holding("SOL", cost: 100, value: 70),
        ])

        XCTAssertEqual(stats.best?.asset.displayTicker, "ETH")
        XCTAssertEqual(stats.worst?.asset.displayTicker, "SOL")
        XCTAssertEqual(stats.assetResults.map(\.asset.displayTicker), ["BTC", "ETH", "SOL"], "Ranked by amount")
        XCTAssertEqual(stats.winners, 2)
        XCTAssertEqual(stats.losers, 1)
    }

    func testDayChangeValuesTodaysQuantitiesAtYesterdaysPrice() {
        let stats = PortfolioStatistics(holdings: [
            holding("BTC", quantity: 2, cost: 100, value: 220, previousPrice: 100),  // 200 → 220
            holding("ETH", quantity: 1, cost: 50, value: 80, previousPrice: nil),  // no baseline: ignored
        ])

        XCTAssertEqual(stats.dayChange?.amount, 20)
        XCTAssertEqual(stats.dayChange?.percentage, Decimal(string: "0.1"))
    }

    func testDayChangeIsNilWithoutAnyBaseline() {
        XCTAssertNil(PortfolioStatistics(holdings: [holding("BTC", cost: 1, value: 2)]).dayChange)
    }

    func testHighAndLowCarryTheirDates() {
        let history = [
            snapshot(1, value: 100, contributions: 100), snapshot(2, value: 300, contributions: 100),
            snapshot(3, value: 50, contributions: 100),
        ]

        let stats = PortfolioStatistics(holdings: [], history: history)

        XCTAssertEqual(stats.high?.value, 300)
        XCTAssertEqual(stats.high?.date, history[1].timestamp)
        XCTAssertEqual(stats.low?.value, 50)
        XCTAssertEqual(stats.low?.date, history[2].timestamp)
    }

    func testActivityCountsTransactionsFeesAndFirstDate() {
        let stats = PortfolioStatistics(
            holdings: [],
            transactions: [
                transaction(.buy, day: 5, fee: 1), transaction(.buy, day: 2, fee: 2),
                transaction(.sell, day: 9, fee: Decimal(string: "0.5")!), transaction(.reward, day: 3),
            ])

        XCTAssertEqual(stats.transactionCount, 4)
        XCTAssertEqual(stats.buyCount, 2)
        XCTAssertEqual(stats.sellCount, 1)
        XCTAssertEqual(stats.feesPaid, Decimal(string: "3.5"))
        XCTAssertEqual(stats.firstTransaction, Date(timeIntervalSince1970: 2 * 86_400))
    }

    func testEmptyPortfolioHasNoExtremesOrPerformers() {
        let stats = PortfolioStatistics(holdings: [])

        XCTAssertEqual(stats.totalPnL, 0)
        XCTAssertNil(stats.best)
        XCTAssertNil(stats.high)
        XCTAssertNil(stats.firstTransaction)
        XCTAssertNil(stats.unrealizedPercent)
    }

    // MARK: - Period change

    func testPeriodChangeIgnoresMoneyDepositedDuringTheRange() {
        // 1000 invested and worth 1000; then 500 more goes in and the whole lot is worth 1650.
        // Only 150 of the 650 rise is profit.
        let range = [
            snapshot(1, value: 1000, contributions: 1000), snapshot(2, value: 1500, contributions: 1500),
        ]

        let change = PortfolioPeriodChange(snapshots: range, currentValue: 1650)

        XCTAssertEqual(change?.profit, 150)
        // Capital in play: 1000 at the start plus the 500 deposited.
        XCTAssertEqual(change?.percentage, Decimal(string: "0.1"))
        XCTAssertEqual(change?.direction, .up)
    }

    func testPeriodChangeReportsALoss() {
        let range = [snapshot(1, value: 200, contributions: 200)]

        let change = PortfolioPeriodChange(snapshots: range, currentValue: 150)

        XCTAssertEqual(change?.profit, -50)
        XCTAssertEqual(change?.percentage, Decimal(string: "-0.25"))
        XCTAssertEqual(change?.direction, .down)
    }

    func testPeriodChangeIsNilWithoutHistoryAndHasNoPercentageFromNothing() {
        XCTAssertNil(PortfolioPeriodChange(snapshots: [], currentValue: 100))
        let fromZero = PortfolioPeriodChange(snapshots: [snapshot(1, value: 0, contributions: 0)], currentValue: 10)
        XCTAssertEqual(fromZero?.profit, 10)
        XCTAssertNil(fromZero?.percentage)
    }

    // MARK: - History range and formatter

    func testRangeFilterKeepsOnlyRecentSnapshots() {
        let now = Date(timeIntervalSince1970: 100 * 86_400)
        let points = [
            snapshot(10, value: 1, contributions: 1), snapshot(95, value: 1, contributions: 1),
            snapshot(99, value: 1, contributions: 1),
        ]

        XCTAssertEqual(PortfolioHistoryRange.oneWeek.filter(points, now: now).count, 2)
        XCTAssertEqual(PortfolioHistoryRange.all.filter(points, now: now).count, 3)
    }

    func testFormatterSignsAndMasksAmounts() {
        let open = PortfolioValueFormatter(currency: .USD, privacy: false)
        let hidden = PortfolioValueFormatter(currency: .USD, privacy: true)

        XCTAssertTrue(open.signedMoney(12).hasPrefix("+"))
        XCTAssertTrue(open.signedMoney(-12).hasPrefix("−"))
        XCTAssertFalse(open.signedMoney(0).hasPrefix("+"))
        XCTAssertEqual(hidden.signedMoney(12), PortfolioValueFormatter.mask)
        XCTAssertEqual(hidden.money(12), PortfolioValueFormatter.mask)
        XCTAssertEqual(hidden.quantity(1, sign: .plus), PortfolioValueFormatter.mask)
        XCTAssertNotNil(open.tone(5))
        XCTAssertNil(open.tone(0))
        XCTAssertNil(hidden.tone(5), "A color would give the hidden sign away")
        XCTAssertTrue(open.quantity(2, sign: .minus).hasPrefix("−"))
    }

    func testSubUnitPricesKeepTheirDecimals() {
        let cheap = PortfolioCurrency.USD.formatPrice(Decimal(string: "0.00002")!)
        XCTAssertTrue(cheap.contains("00002"), "got \(cheap)")
        // From one unit up, a price reads like any other amount.
        XCTAssertEqual(PortfolioCurrency.USD.formatPrice(1234.5), PortfolioCurrency.USD.format(1234.5))
        let half = Decimal(string: "0.5")!
        XCTAssertEqual(PortfolioCurrency.BTC.formatPrice(half), PortfolioCurrency.BTC.format(half))
    }

    func testCompactAxisLabelsPutTheSuffixOnTheNumber() {
        let label = PortfolioCurrency.USD.formatCompact(135_840)
        XCTAssertTrue(label.hasSuffix("K"), "got \(label)")
        XCTAssertFalse(label.contains("$"))
        XCTAssertTrue(PortfolioCurrency.EUR.formatCompact(2_500_000).hasSuffix("M"))
    }
}
