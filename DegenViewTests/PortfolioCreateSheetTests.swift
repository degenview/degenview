import XCTest

@testable import DegenView

final class PortfolioCreateSheetTests: XCTestCase {
    func testEveryCurrencyHasAGlyphAndAName() {
        for currency in PortfolioCurrency.allCases {
            XCTAssertFalse(currency.glyph.isEmpty, "\(currency)")
            XCTAssertFalse(currency.displayName.isEmpty, "\(currency)")
        }
        XCTAssertEqual(PortfolioCurrency.BTC.displayName, "Bitcoin")
        XCTAssertEqual(PortfolioCurrency.BTC.glyph, "₿")
        XCTAssertEqual(PortfolioCurrency.EUR.glyph, "€")
    }

    func testNamesAreTrimmedAndCapped() {
        XCTAssertEqual(PortfolioNameCheck.normalized("  Long-term  "), "Long-term")
        XCTAssertEqual(
            PortfolioNameCheck.normalized(String(repeating: "a", count: 80)).count, PortfolioNameCheck.maxLength)
    }

    func testBlankNamesAreInvalid() {
        XCTAssertFalse(PortfolioNameCheck.isValid(""))
        XCTAssertFalse(PortfolioNameCheck.isValid("  \n "))
        XCTAssertTrue(PortfolioNameCheck.isValid(" Main "))
    }

    func testDuplicateCheckIgnoresCaseAndSpacing() {
        XCTAssertTrue(PortfolioNameCheck.isDuplicate(" main ", among: ["Main", "Trading"]))
        XCTAssertFalse(PortfolioNameCheck.isDuplicate("Savings", among: ["Main", "Trading"]))
        XCTAssertFalse(PortfolioNameCheck.isDuplicate("  ", among: ["", "Main"]), "A blank name is invalid, not a duplicate")
    }
}
