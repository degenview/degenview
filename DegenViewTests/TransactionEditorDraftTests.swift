import XCTest

@testable import DegenView

final class TransactionEditorDraftTests: XCTestCase {
    private let us = Locale(identifier: "en_US")
    private let german = Locale(identifier: "de_DE")

    private func asset() -> PortfolioAsset {
        PortfolioAsset(key: "Binance:BTC/USDT", symbol: "BTC/USDT", name: "BTC/USDT", source: .binance)
    }

    private func base(
        type: PortfolioTransactionType = .buy, quantity: Decimal = 0, price: Decimal? = nil, fee: Decimal = 0
    ) -> PortfolioTransaction {
        PortfolioTransaction(
            portfolioID: UUID(), asset: asset(), type: type, quantity: quantity, price: price, fee: fee)
    }

    private func draft(_ transaction: PortfolioTransaction? = nil, locale: Locale? = nil) -> TransactionEditorDraft {
        TransactionEditorDraft(transaction: transaction ?? base(), locale: locale ?? us)
    }

    // MARK: Price and total

    func testTotalFollowsPriceAndQuantity() {
        var draft = draft()
        draft.quantityText = "2"
        draft.setPrice("100")
        XCTAssertEqual(draft.total, 200)
        XCTAssertEqual(draft.displayedTotal, "200.00")
        draft.quantityText = "3"
        XCTAssertEqual(draft.displayedTotal, "300.00")
    }

    func testPriceFollowsTotalOnceTotalIsTyped() {
        var draft = draft()
        draft.quantityText = "4"
        draft.setPrice("100")
        draft.setTotal("500")
        XCTAssertEqual(draft.lastEdited, .total)
        XCTAssertEqual(draft.price, 125)
        XCTAssertEqual(draft.displayedPrice, "125.00")
        // Changing the quantity now moves the price, not the total the user typed.
        draft.quantityText = "5"
        XCTAssertEqual(draft.price, 100)
        XCTAssertEqual(draft.total, 500)
    }

    func testTypingInPriceTakesTheSourceBackFromTotal() {
        var draft = draft()
        draft.quantityText = "2"
        draft.setTotal("300")
        draft.setPrice("10")
        XCTAssertEqual(draft.lastEdited, .price)
        XCTAssertEqual(draft.total, 20)
    }

    func testSettingAFieldToItsDisplayedTextDoesNotTakeOverSourceOfTruth() {
        var draft = draft()
        draft.quantityText = "2"
        draft.setPrice("50")
        draft.setTotal(draft.displayedTotal)
        XCTAssertEqual(draft.lastEdited, .price)
    }

    func testTotalWithoutQuantityLeavesPriceUnknown() {
        var draft = draft()
        draft.setTotal("500")
        XCTAssertNil(draft.price)
        XCTAssertEqual(draft.displayedPrice, "")
    }

    func testMarketPriceFillsAnEmptyPriceAndTotalFollows() {
        var draft = draft()
        draft.quantityText = "2"
        draft.prefillPrice(Decimal(string: "67432.123456789")!)
        XCTAssertEqual(draft.displayedPrice, "67,432.12")
        XCTAssertEqual(draft.lastEdited, .price)
        XCTAssertNotNil(draft.total)
    }

    func testMarketPriceNeverOverwritesWhatWasTyped() {
        var typedPrice = draft()
        typedPrice.setPrice("100")
        typedPrice.prefillPrice(200)
        XCTAssertEqual(typedPrice.price, 100)

        var typedTotal = draft()
        typedTotal.quantityText = "2"
        typedTotal.setTotal("500")
        typedTotal.prefillPrice(200)
        XCTAssertEqual(typedTotal.price, 250)

        var existing = draft(base(quantity: 1, price: 50))
        existing.prefillPrice(200)
        XCTAssertEqual(existing.price, 50)
    }

    func testMarketPriceMustBePositive() {
        var draft = draft()
        draft.prefillPrice(0)
        XCTAssertEqual(draft.displayedPrice, "")
    }

    func testMarketPriceUsesTheLocaleSeparator() {
        var draft = draft(locale: german)
        draft.prefillPrice(Decimal(string: "1.5")!)
        XCTAssertEqual(draft.displayedPrice, "1,50")
    }

    func testPriceAndTotalShareOneLocaleAndReadAsMoney() {
        var draft = draft(locale: german)
        draft.quantityText = "0,001"
        draft.prefillPrice(Decimal(string: "85152.15")!)
        XCTAssertEqual(draft.displayedPrice, "85.152,15")
        XCTAssertEqual(draft.displayedTotal, "85,15")
        // The grouped text reads back as the same number.
        XCTAssertEqual(draft.price, Decimal(string: "85152.15"))
        XCTAssertEqual(draft.total, Decimal(string: "85.15215"))
    }

    func testSmallPricesKeepTheirSignificantDigits() {
        var draft = draft()
        draft.prefillPrice(Decimal(string: "0.000002784")!)
        XCTAssertEqual(draft.displayedPrice, "0.000002784")
    }

    func testOpeningAnExistingTransactionDoesNotChangeItsPrice() throws {
        let original = base(type: .buy, quantity: 1, price: Decimal(string: "85152.1234")!)
        let made = try XCTUnwrap(draft(original).makeTransaction(from: original))
        XCTAssertEqual(made.price, original.price)
        XCTAssertEqual(made.quantity, original.quantity)
    }

    // MARK: Parsing

    func testCommaDecimalsParseInACommaLocale() {
        var draft = draft(locale: german)
        draft.quantityText = "1,5"
        draft.setPrice("2000,50")
        XCTAssertEqual(draft.quantity, Decimal(string: "1.5"))
        XCTAssertEqual(draft.price, Decimal(string: "2000.5"))
        XCTAssertEqual(draft.total, Decimal(string: "3000.75"))
    }

    func testExistingTransactionFillsTheFields() {
        let draft = draft(base(type: .sell, quantity: Decimal(string: "0.25")!, price: 40_000, fee: 5))
        XCTAssertEqual(draft.quantityText, "0.25")
        XCTAssertEqual(draft.displayedPrice, "40,000.00")
        XCTAssertEqual(draft.displayedTotal, "10,000.00")
        XCTAssertEqual(draft.feeText, "5")
        XCTAssertEqual(draft.category, .sell)
    }

    // MARK: Validation

    func testFreshDraftHasNoIssuesButCannotSubmit() {
        let draft = draft()
        XCTAssertTrue(draft.issues.isEmpty)
        XCTAssertFalse(draft.canSubmit)
    }

    func testQuantityMustBePositiveNumber() {
        var draft = draft()
        draft.quantityText = "abc"
        XCTAssertEqual(draft.issues[.quantity], "Enter a number.")
        draft.quantityText = "0"
        XCTAssertEqual(draft.issues[.quantity], "Must be greater than zero.")
        draft.quantityText = "-1"
        XCTAssertEqual(draft.issues[.quantity], "Can't be negative.")
        draft.quantityText = "1"
        XCTAssertNil(draft.issues[.quantity])
    }

    func testBuyAndSellNeedAPrice() {
        for type in [PortfolioTransactionType.buy, .sell] {
            var draft = draft(base(type: type))
            draft.quantityText = "1"
            XCTAssertFalse(draft.canSubmit, "\(type)")
            draft.setPrice("100")
            XCTAssertTrue(draft.canSubmit, "\(type)")
        }
    }

    func testOtherTypesDoNotNeedAPrice() {
        for type in [PortfolioTransactionType.transferIn, .transferOut, .reward, .airdrop, .fee, .adjustment] {
            var draft = draft(base(type: type))
            draft.quantityText = "1"
            XCTAssertTrue(draft.canSubmit, "\(type)")
        }
    }

    func testBadFeeBlocksSubmitting() {
        var draft = draft(base(type: .reward))
        draft.quantityText = "1"
        draft.feeText = "x"
        XCTAssertEqual(draft.issues[.fee], "Enter a number.")
        XCTAssertFalse(draft.canSubmit)
        draft.feeText = "-2"
        XCTAssertEqual(draft.issues[.fee], "Can't be negative.")
        draft.feeText = ""
        XCTAssertTrue(draft.canSubmit)
    }

    func testPriceMayBeZeroForAnAirdropButNotNegative() {
        var draft = draft(base(type: .airdrop))
        draft.quantityText = "10"
        draft.setPrice("0")
        XCTAssertNil(draft.issues[.price])
        draft.setPrice("-1")
        XCTAssertEqual(draft.issues[.price], "Can't be negative.")
    }

    // MARK: Types

    func testEveryTypeBelongsToExactlyOneCategory() {
        for type in PortfolioTransactionType.allCases {
            let owners = TransactionEditorDraft.Category.allCases.filter { $0.types.contains(type) }
            XCTAssertEqual(owners.count, 1, "\(type)")
            XCTAssertEqual(TransactionEditorDraft.Category(type), owners.first)
        }
    }

    func testSelectingACategoryKeepsATypeAlreadyInIt() {
        var draft = draft(base(type: .airdrop))
        draft.select(.income)
        XCTAssertEqual(draft.type, .airdrop)
        draft.select(.transfer)
        XCTAssertEqual(draft.type, .transferIn)
        draft.select(.sell)
        XCTAssertEqual(draft.type, .sell)
    }

    // MARK: Summary

    func testBuySummaryAddsTheFee() {
        var draft = draft()
        draft.quantityText = "2"
        draft.setPrice("100")
        draft.feeText = "1.5"
        let summary = draft.summary
        XCTAssertEqual(summary?.value, 200)
        XCTAssertEqual(summary?.fee, Decimal(string: "1.5"))
        XCTAssertEqual(summary?.net?.label, "Total cost")
        XCTAssertEqual(summary?.net?.amount, Decimal(string: "201.5"))
    }

    func testSellSummarySubtractsTheFee() {
        var draft = draft(base(type: .sell))
        draft.quantityText = "2"
        draft.setPrice("100")
        draft.feeText = "1"
        XCTAssertEqual(draft.summary?.net?.label, "Net proceeds")
        XCTAssertEqual(draft.summary?.net?.amount, 199)
    }

    func testSummaryIsAbsentWithoutAPriceOrForPricelessTypes() {
        var draft = draft()
        draft.quantityText = "2"
        XCTAssertNil(draft.summary)
        draft.setPrice("100")
        draft.type = .transferOut
        XCTAssertNil(draft.summary)
    }

    // MARK: Output

    func testMakeTransactionKeepsIdentityAndAppliesEdits() throws {
        let original = base(type: .buy, quantity: 1, price: 100)
        var draft = draft(original)
        draft.quantityText = "3"
        draft.setPrice("110")
        draft.feeText = "2"
        draft.notes = "DCA"
        let now = Date(timeIntervalSince1970: 1_700_000_000)
        let made = try XCTUnwrap(draft.makeTransaction(from: original, now: now))
        XCTAssertEqual(made.id, original.id)
        XCTAssertEqual(made.portfolioID, original.portfolioID)
        XCTAssertEqual(made.asset, original.asset)
        XCTAssertEqual(made.source, original.source)
        XCTAssertEqual(made.createdAt, original.createdAt)
        XCTAssertEqual(made.quantity, 3)
        XCTAssertEqual(made.price, 110)
        XCTAssertEqual(made.fee, 2)
        XCTAssertEqual(made.notes, "DCA")
        XCTAssertEqual(made.updatedAt, now)
    }

    func testPriceIsDroppedWhenTheTypeNoLongerUsesOne() throws {
        let original = base(type: .buy, quantity: 1, price: 100)
        var draft = draft(original)
        draft.select(.transfer)
        draft.type = .transferOut
        let made = try XCTUnwrap(draft.makeTransaction(from: original))
        XCTAssertNil(made.price)
    }

    func testMakeTransactionIsNilWhileInvalid() {
        XCTAssertNil(draft().makeTransaction(from: base()))
    }
}
