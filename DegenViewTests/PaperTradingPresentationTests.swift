import SwiftUI
import XCTest

@testable import DegenView

final class PaperTradingPresentationTests: XCTestCase {
    private let us = Locale(identifier: "en_US")
    private let germany = Locale(identifier: "de_DE")

    /// Decimals built from strings: a float literal such as `1234.56` is not the same `Decimal`.
    private func dec(_ string: String) -> Decimal {
        Decimal(string: string, locale: Locale(identifier: "en_US_POSIX"))!
    }

    private var crypto: PaperInstrument {
        .init(
            key: "test:BTC", symbol: "BTCUSDT", displayName: "BTC/USDT", source: .binance, assetClass: .crypto,
            quoteCurrency: .USD, tickSize: dec("0.01"), minimumQuantity: dec("0.001"),
            quantityIncrement: dec("0.001"), contractMultiplier: 1, pointValue: 1)
    }

    private func draft(
        side: PaperOrderSide = .buy, type: PaperOrderType = .market, quantity: String = "",
        funds: Decimal? = 1_000, leverage: Decimal = 1, existing: Decimal = 0
    ) -> PaperOrderTicketDraft {
        var draft = PaperOrderTicketDraft(instrument: crypto)
        draft.bid = 99
        draft.ask = 100
        draft.last = 99.5
        draft.availableFunds = funds
        draft.leverage = leverage
        draft.existingSignedQuantity = existing
        draft.side = side
        draft.type = type
        draft.quantityText = quantity
        draft.locale = us
        return draft
    }

    // MARK: PaperDecimalInput

    func testParseReadsEitherSeparatorConvention() {
        XCTAssertEqual(PaperDecimalInput.parse("1,234.56", locale: us), dec("1234.56"))
        XCTAssertEqual(PaperDecimalInput.parse("1.5", locale: us), 1.5)
        XCTAssertEqual(PaperDecimalInput.parse("1,5", locale: us), 1.5)
        XCTAssertEqual(PaperDecimalInput.parse("1,234", locale: us), 1234)
        XCTAssertEqual(PaperDecimalInput.parse("1,234,567", locale: us), 1_234_567)
        XCTAssertEqual(PaperDecimalInput.parse("0,500", locale: us), 0.5)
        XCTAssertEqual(PaperDecimalInput.parse(".5", locale: us), 0.5)
        XCTAssertEqual(PaperDecimalInput.parse(" 12 ", locale: us), 12)

        XCTAssertEqual(PaperDecimalInput.parse("1.234,56", locale: germany), dec("1234.56"))
        XCTAssertEqual(PaperDecimalInput.parse("1,5", locale: germany), 1.5)
        XCTAssertEqual(PaperDecimalInput.parse("1.234", locale: germany), 1234)
        XCTAssertEqual(PaperDecimalInput.parse("0.5", locale: germany), 0.5)
    }

    func testParseRejectsNonNumbers() {
        for text in ["", "  ", "abc", "12abc", "-", ".", "1e5", "1-2"] {
            XCTAssertNil(PaperDecimalInput.parse(text, locale: us), "\(text) should not parse")
        }
    }

    func testTextRoundTripsThroughParse() {
        XCTAssertEqual(PaperDecimalInput.text(1234.5, locale: us), "1234.5")
        XCTAssertEqual(PaperDecimalInput.text(1.234, locale: germany), "1,234")
        for locale in [us, germany] {
            for string in ["1.234", "0.00012345", "67432.19", "5", "1000"] {
                let value = dec(string)
                let text = PaperDecimalInput.text(value, locale: locale)
                XCTAssertEqual(PaperDecimalInput.parse(text, locale: locale), value, "\(text) in \(locale.identifier)")
            }
        }
    }

    // MARK: PaperOrderTicketDraft

    func testMarketBuyUsesAskAndLeverage() {
        let draft = draft(quantity: "2", leverage: 2)
        XCTAssertEqual(draft.marketPrice, 100)
        XCTAssertEqual(draft.notional, 200)
        XCTAssertEqual(draft.requiredMargin, 100)
        XCTAssertTrue(draft.canSubmit)
        XCTAssertEqual(draft.spread, 1)
    }

    func testMarketSellUsesBid() {
        XCTAssertEqual(draft(side: .sell, quantity: "1").marketPrice, 99)
    }

    func testInsufficientFundsBlocksSubmit() {
        let draft = draft(quantity: "20")
        XCTAssertEqual(draft.requiredMargin, 2_000)
        XCTAssertTrue(draft.exceedsFunds)
        XCTAssertFalse(draft.canSubmit)
    }

    func testMarginIsOnlyChargedForNewExposure() {
        // Selling 3 of a 5 long closes exposure; no margin is needed.
        let reduce = draft(side: .sell, quantity: "3", funds: 0, existing: 5)
        XCTAssertEqual(reduce.requiredMargin, 0)
        XCTAssertFalse(reduce.flipsPosition)
        XCTAssertTrue(reduce.canSubmit)

        // Selling 5 of a 2 long closes it and opens 3 short at the bid.
        let flip = draft(side: .sell, quantity: "5", existing: 2)
        XCTAssertTrue(flip.flipsPosition)
        XCTAssertEqual(flip.requiredMargin, 3 * 99)

        // Buying more of a long adds exposure on the whole size.
        XCTAssertEqual(draft(quantity: "1", existing: 2).requiredMargin, 100)
    }

    func testQuantityIssuesFollowTheInstrumentsIncrements() {
        XCTAssertTrue(draft(quantity: "").missingFields.contains(.quantity))
        XCTAssertNil(draft(quantity: "").issues[.quantity])
        XCTAssertNotNil(draft(quantity: "0").issues[.quantity])
        XCTAssertNotNil(draft(quantity: "-1").issues[.quantity])
        XCTAssertNotNil(draft(quantity: "abc").issues[.quantity])
        XCTAssertNotNil(draft(quantity: "0.0005").issues[.quantity], "below the minimum")
        XCTAssertNotNil(draft(quantity: "0.0015").issues[.quantity], "off the increment")
        XCTAssertNil(draft(quantity: "0.002").issues[.quantity])
        XCTAssertFalse(draft(quantity: "").canSubmit)
    }

    func testPriceFieldsAreRequiredOnlyForTheirOrderType() {
        var limit = draft(type: .limit, quantity: "1")
        XCTAssertEqual(limit.missingFields, [.limitPrice])
        limit.limitPriceText = "98.5"
        XCTAssertTrue(limit.missingFields.isEmpty)
        XCTAssertEqual(limit.valuationPrice, 98.5)
        limit.limitPriceText = "98.555"
        XCTAssertNotNil(limit.issues[.limitPrice], "off the tick")

        var stopLimit = draft(type: .stopLimit, quantity: "1")
        XCTAssertEqual(stopLimit.missingFields, [.limitPrice, .stopPrice])
        stopLimit.limitPriceText = "101"
        stopLimit.stopPriceText = "100"
        XCTAssertTrue(stopLimit.canSubmit)
        XCTAssertEqual(draft(type: .market, quantity: "1").missingFields, [])
    }

    func testProtectionMustSitOnTheRightSideOfTheEntry() {
        var buy = draft(quantity: "1")  // enters at the ask, 100
        buy.takeProfitText = "110"
        buy.stopLossText = "90"
        XCTAssertTrue(buy.issues.isEmpty)
        buy.takeProfitText = "95"
        buy.stopLossText = "105"
        XCTAssertNotNil(buy.issues[.takeProfit])
        XCTAssertNotNil(buy.issues[.stopLoss])

        var sell = draft(side: .sell, quantity: "1")  // enters at the bid, 99
        sell.takeProfitText = "90"
        sell.stopLossText = "110"
        XCTAssertTrue(sell.issues.isEmpty)
        sell.takeProfitText = "110"
        XCTAssertNotNil(sell.issues[.takeProfit])
    }

    func testFundsFractionRoundsDownToTheIncrement() {
        // 1000 × 0.5 × 2× leverage ÷ 100 = 10
        XCTAssertEqual(draft(leverage: 2).quantity(forFundsFraction: 0.5), 10)
        // 1000 ÷ 300 = 3.333… → 3.333
        var limit = draft(type: .limit)
        limit.limitPriceText = "300"
        XCTAssertEqual(limit.quantity(forFundsFraction: 1), dec("3.333"))
        // Too little to reach the minimum size.
        XCTAssertNil(draft(funds: 0.01).quantity(forFundsFraction: 1))
        XCTAssertNil(draft(funds: nil).quantity(forFundsFraction: 1))
        XCTAssertNil(draft(funds: 0).quantity(forFundsFraction: 1))
    }

    func testEstimatedFeeFollowsTheCommissionModel() {
        var draft = draft(quantity: "2")
        XCTAssertNil(draft.estimatedFee)
        draft.commission = .fixedPerOrder(1.5)
        XCTAssertEqual(draft.estimatedFee, 1.5)
        draft.commission = .percentage(0.1)
        XCTAssertEqual(draft.estimatedFee, Decimal(string: "0.2"))
        draft.commission = .perContract(3)
        XCTAssertEqual(draft.estimatedFee, 6)
    }

    func testRequestCarriesOnlyTheFieldsOfItsType() throws {
        let account = UUID()
        var draft = draft(type: .limit, quantity: "1")
        draft.limitPriceText = "98"
        draft.stopPriceText = "97"
        draft.takeProfitText = "120"
        let limit = try XCTUnwrap(draft.request(accountID: account))
        XCTAssertEqual(limit.limitPrice, 98)
        XCTAssertNil(limit.stopPrice)
        XCTAssertEqual(limit.takeProfit, 120)
        XCTAssertNil(limit.stopLoss)
        XCTAssertEqual(limit.accountID, account)

        draft.type = .market
        let market = try XCTUnwrap(draft.request(accountID: account))
        XCTAssertNil(market.limitPrice)
        XCTAssertNil(market.stopPrice)

        draft.quantityText = "x"
        XCTAssertNil(draft.request(accountID: account))
    }

    // MARK: Formatting

    func testSignedPercent() {
        XCTAssertEqual(PaperTradingFormatter.signedPercent(0.0123, locale: us), "+1.23%")
        XCTAssertEqual(PaperTradingFormatter.signedPercent(-0.041, locale: us), "-4.10%")
        XCTAssertEqual(PaperTradingFormatter.signedPercent(0, locale: us), "0.00%")
    }

    func testCachedFormattersMatchAcrossCallsAndLocales() {
        for _ in 0..<2 {
            XCTAssertEqual(PaperTradingFormatter.money(1234.567, currency: .USD, locale: us), "$1,234.57")
            XCTAssertEqual(PaperTradingFormatter.money(1234.567, currency: .JPY, locale: us), "¥1,235")
            XCTAssertEqual(PaperTradingFormatter.signedMoney(12.5, currency: .USD, locale: us), "+$12.50")
        }
        XCTAssertEqual(PaperTradingFormatter.price(1234.5, instrument: crypto, locale: us), "1,234.5")
        XCTAssertEqual(PaperTradingFormatter.price(1234.5, instrument: crypto, locale: germany), "1.234,5")
        XCTAssertEqual(PaperTradingFormatter.quantity(0.12345, instrument: crypto, locale: us), "0.123")
    }

    // MARK: Labels and tones

    func testLabelsAreHumanReadable() {
        XCTAssertEqual(PaperOrderType.stopLimit.label, "Stop Limit")
        XCTAssertEqual(PaperOrderStatus.partiallyFilled.label, "Partial")
        XCTAssertEqual(PaperOrderStatus.partiallyFilled.tone, .warning)
        XCTAssertEqual(PaperOrderStatus.rejected.tone, .bad)
        XCTAssertEqual(PaperOrderEventKind.partiallyFilled.label, "Partial fill")
        XCTAssertEqual(PaperOrderEventKind.filled.tone, .good)
        XCTAssertEqual(PaperOrderRole.takeProfit.badgeText, "TP")
        XCTAssertEqual(PaperManagerTab.accountHistory.title, "Trades")
        XCTAssertEqual(PaperManagerTab.accountHistory.rawValue, "Account History")
        XCTAssertEqual(crypto.baseSymbol, "BTC")
    }

    func testBuyIsGreenAndSellIsRed() {
        XCTAssertEqual(PaperOrderSide.buy.tint, .green)
        XCTAssertEqual(PaperOrderSide.sell.tint, .red)
        XCTAssertEqual(PaperPositionSide.long.tint, .green)
        XCTAssertEqual(PaperPositionSide.short.tint, .red)
        XCTAssertEqual(PaperTradingStyle.pnl(1), .green)
        XCTAssertEqual(PaperTradingStyle.pnl(-1), .red)
        XCTAssertEqual(PaperTradingStyle.pnl(0), .secondary)
        XCTAssertEqual(PaperTradingStyle.marginBuffer(0.05), .red)
        XCTAssertEqual(PaperTradingStyle.marginBuffer(0.2), .orange)
        XCTAssertEqual(PaperTradingStyle.marginBuffer(0.6), .primary)
    }

    func testTradeStats() {
        func trade(gross: Decimal, commission: Decimal) -> PaperClosedTrade {
            .init(
                id: UUID(), accountID: UUID(), instrument: crypto, side: .long, entryTimestamp: Date(),
                exitTimestamp: Date(), entryPrice: 100, exitPrice: 101, quantity: 1, grossPnL: gross,
                commission: commission)
        }
        XCTAssertNil([PaperClosedTrade]().winRate)
        let trades = [
            trade(gross: 10, commission: 1), trade(gross: 5, commission: 1), trade(gross: -4, commission: 1),
            trade(gross: 1, commission: 1),  // breaks even before commission only
        ]
        XCTAssertEqual(trades.netPnL, 10 - 1 + 5 - 1 - 4 - 1 + 1 - 1)
        XCTAssertEqual(trades.winRate, 0.5)
    }

    // MARK: Store marks

    @MainActor
    func testStoreMarksPositionsAtTheClosingSide() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = PaperTradingStore(database: try AppDatabase(path: directory.appendingPathComponent("t.sqlite").path))
        await store.connect()
        let stock = PaperInstrument(
            key: "test:XYZ", symbol: "XYZ", displayName: "XYZ", source: .alpaca, assetClass: .stock,
            quoteCurrency: .USD, tickSize: 1, minimumQuantity: 1, quantityIncrement: 1, contractMultiplier: 1,
            pointValue: 1)
        await store.process(instrument: stock, bid: 99, ask: 101, last: 100, timestamp: Date())
        let accountID = try XCTUnwrap(store.selectedAccount?.id)
        let accepted = await store.submit(
            .init(accountID: accountID, instrument: stock, side: .buy, type: .market, quantity: 10))
        XCTAssertTrue(accepted)
        let position = try XCTUnwrap(store.positions.first)
        XCTAssertEqual(position.averageEntryPrice, 101)

        await store.process(instrument: stock, bid: 111, ask: 113, last: 112, timestamp: Date())
        XCTAssertEqual(store.mark(for: position), 111, "a long closes at the bid")
        XCTAssertEqual(store.unrealizedPnL(for: position), 100)
        let ratio = try XCTUnwrap(store.returnRatio(for: position))
        XCTAssertEqual(ratio.doubleValue, 100.0 / 1010.0, accuracy: 0.0001)
        XCTAssertEqual(store.metrics?.unrealizedPnL, 100, "the view's figure matches the engine's")
    }
}
