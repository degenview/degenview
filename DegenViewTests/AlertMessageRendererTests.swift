import XCTest

@testable import DegenView

final class AlertMessageRendererTests: XCTestCase {
    private let time = Date(timeIntervalSince1970: 1_714_571_100)  // 2024-05-01T13:45:00Z
    private lazy var context = AlertMessageContext(
        ticker: "BTCUSDT", exchange: "Binance", interval: "60", open: 99_000, high: 101_000.25, low: 98_500,
        close: 100_000.5, volume: 1234.5, time: time, now: Date(timeIntervalSince1970: 1_714_571_160))

    private func render(_ template: String) -> String {
        AlertMessageRenderer.render(template, context: context)
    }

    func testResolvesEveryDocumentedPlaceholder() {
        XCTAssertEqual(render("{{ticker}}"), "BTCUSDT")
        XCTAssertEqual(render("{{exchange}}"), "Binance")
        XCTAssertEqual(render("{{interval}}"), "60")
        XCTAssertEqual(render("{{open}}"), "99000")
        XCTAssertEqual(render("{{high}}"), "101000.25")
        XCTAssertEqual(render("{{low}}"), "98500")
        XCTAssertEqual(render("{{close}}"), "100000.5")
        XCTAssertEqual(render("{{volume}}"), "1234.5")
        XCTAssertEqual(render("{{time}}"), "2024-05-01T13:45:00Z")
        XCTAssertEqual(render("{{timenow}}"), "2024-05-01T13:46:00Z")
        XCTAssertEqual(Set(AlertMessageRenderer.supportedPlaceholders).count, 10)
    }

    func testSmallPricesHaveNoExponentOrGrouping() {
        let tiny = AlertMessageContext(close: 0.00000278)
        XCTAssertEqual(AlertMessageRenderer.render("{{close}}", context: tiny), "0.00000278")
        XCTAssertEqual(
            AlertMessageRenderer.render("{{close}}", context: AlertMessageContext(close: 1_234_567.5)), "1234567.5")
    }

    func testSurroundingTextIsUntouched() {
        XCTAssertEqual(render("BUY {{ticker}} @ {{close}}!"), "BUY BTCUSDT @ 100000.5!")
        XCTAssertEqual(render("no placeholders"), "no placeholders")
        XCTAssertEqual(render(""), "")
    }

    func testMalformedTokensDoNotMutateUnrelatedText() {
        XCTAssertEqual(render("foo{{close"), "foo{{close")
        XCTAssertEqual(render("{{close"), "{{close")
        XCTAssertEqual(render("close}}"), "close}}")
        XCTAssertEqual(render("{close}"), "{close}")
        XCTAssertEqual(render("{{ close }}"), "{{ close }}")
        XCTAssertEqual(render("{{}}"), "{{}}")
        XCTAssertEqual(render("{{{{close}}"), "{{100000.5")
        XCTAssertEqual(render("{{close}}}}"), "100000.5}}")
    }

    func testUnknownPlaceholdersStayLiteral() {
        XCTAssertEqual(render("{{plot_0}} {{ticker}}"), "{{plot_0}} BTCUSDT")
        XCTAssertEqual(render("{{strategy.order.action}}"), "{{strategy.order.action}}")
    }

    func testMissingValuesAreNotFabricated() {
        let bare = AlertMessageContext(ticker: "ETHUSDT", now: Date(timeIntervalSince1970: 0))
        XCTAssertEqual(
            AlertMessageRenderer.render("{{ticker}} {{close}} {{volume}} {{time}}", context: bare),
            "ETHUSDT {{close}} {{volume}} {{time}}")
    }

    func testSubstitutedValuesAreNeverRescanned() {
        let sneaky = AlertMessageContext(ticker: "{{close}}", close: 5)
        XCTAssertEqual(AlertMessageRenderer.render("{{ticker}}", context: sneaky), "{{close}}")
    }

    func testUnicodeTextSurvives() {
        XCTAssertEqual(render("価格 🚀 {{ticker}} 🚀"), "価格 🚀 BTCUSDT 🚀")
    }

    func testIntervalNamesMapToTradingViewStyle() {
        let expected = [
            "1m": "1", "5m": "5", "15m": "15", "1h": "60", "4h": "240", "1d": "D", "1w": "W", "1M": "M",
            "weird": "weird",
        ]
        for (input, output) in expected {
            XCTAssertEqual(AlertMessageRenderer.tradingViewInterval(input), output, input)
        }
    }
}
