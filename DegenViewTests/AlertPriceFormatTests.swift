import XCTest

@testable import DegenView

final class AlertPriceFormatTests: XCTestCase {
    private let locale = Locale(identifier: "en_US")

    private func usd(_ text: String) -> String {
        PortfolioCurrency.USD.formatAlertPrice(Decimal(string: text)!, locale: locale)
    }

    func testLargePricesLoseTheirConversionTail() {
        XCTAssertEqual(usd("67432.18734921"), "$67,432.19")
        XCTAssertEqual(usd("1000"), "$1,000.00")
    }

    func testPricesFromOneUnitKeepUpToFourDecimals() {
        XCTAssertEqual(usd("1.5"), "$1.50")
        XCTAssertEqual(usd("1.23456"), "$1.2346")
        XCTAssertEqual(usd("999.99999"), "$1,000.00")
    }

    func testSubUnitPricesKeepFourSignificantFigures() {
        XCTAssertEqual(usd("0.5"), "$0.50")
        XCTAssertEqual(usd("0.05123456"), "$0.05123")
        XCTAssertEqual(usd("0.00000278"), "$0.00000278")
        XCTAssertEqual(usd("0.000012345678"), "$0.00001235")
    }

    func testZeroAndAbsurdlySmallPricesStayBounded() {
        XCTAssertEqual(usd("0"), "$0.00")
        XCTAssertEqual(usd("0.000000000123"), "$0.000000000123")
        XCTAssertEqual(usd("0.00000000000000001"), "$0.00", "Below twelve decimals it reads as zero")
    }

    func testOtherCurrenciesUseTheirOwnSign() {
        XCTAssertEqual(PortfolioCurrency.EUR.formatAlertPrice(1234.5, locale: locale), "€1,234.50")
        XCTAssertEqual(PortfolioCurrency.GBP.formatAlertPrice(Decimal(string: "0.00002")!, locale: locale), "£0.00002")
    }

    func testYenHasNoFractionFromOneUnit() {
        XCTAssertEqual(PortfolioCurrency.JPY.formatAlertPrice(Decimal(string: "15234.7")!, locale: locale), "¥15,235")
        XCTAssertEqual(PortfolioCurrency.JPY.formatAlertPrice(Decimal(string: "0.5")!, locale: locale), "¥0.5")
    }

    func testBitcoinTrimsTrailingZeros() {
        XCTAssertEqual(PortfolioCurrency.BTC.formatAlertPrice(2, locale: locale), "BTC 2")
        XCTAssertEqual(
            PortfolioCurrency.BTC.formatAlertPrice(Decimal(string: "0.00123456")!, locale: locale), "BTC 0.001235")
    }

    func testNegativeValuesKeepTheirSign() {
        XCTAssertEqual(usd("-1.23456"), "-$1.2346")
    }
}
