import XCTest

@testable import DegenView

final class WebhookPickerLogicTests: XCTestCase {
    private typealias L = WebhookPickerLogic

    func testHostNeverLeaksPathQueryOrUserInfo() {
        XCTAssertEqual(L.host(of: "https://example.com/hook/abc123secret?token=zzz#frag"), "example.com")
        XCTAssertEqual(L.host(of: "https://user:pass@example.com/x"), "example.com")
        XCTAssertEqual(L.host(of: "http://localhost:8080/h"), "localhost:8080")
        XCTAssertEqual(L.host(of: "  https://example.com/h  "), "example.com")
    }

    func testHostWorksOnSecretTemplates() {
        XCTAssertEqual(L.host(of: "https://example.com/hook?token={{secret}}"), "example.com")
        XCTAssertEqual(L.host(of: "https://example.com/{{ secret }}/run"), "example.com")
    }

    func testHostOfGarbageIsNil() {
        XCTAssertNil(L.host(of: ""))
        XCTAssertNil(L.host(of: "not a url"))
        XCTAssertNil(L.host(of: "example.com/hook"))
    }

    func testSubtitleShowsMethodAndHostOrThePause() {
        XCTAssertEqual(
            L.subtitle(for: WebhookTestFixtures.endpoint(url: "https://bot.example.com/h?t={{secret}}")),
            "POST · bot.example.com")
        XCTAssertEqual(
            L.subtitle(for: WebhookTestFixtures.endpoint(url: "http://localhost:9000/h", method: .put)),
            "PUT · localhost:9000")
        XCTAssertEqual(L.subtitle(for: WebhookTestFixtures.endpoint(enabled: false)), "Paused in Settings")
        XCTAssertEqual(L.subtitle(for: WebhookTestFixtures.endpoint(url: "")), "POST")
        XCTAssertFalse(
            L.subtitle(for: WebhookTestFixtures.endpoint(url: "https://e.com/SECRETPATH")).contains("SECRETPATH"))
    }

    func testSummaryCountsSelectedDanglingAndPaused() {
        let a = WebhookTestFixtures.endpoint("A")
        let b = WebhookTestFixtures.endpoint("B", enabled: false)
        let c = WebhookTestFixtures.endpoint("C")
        let all = [a, b, c]

        XCTAssertEqual(
            L.summary(selection: [], endpoints: all), .init(selectedCount: 0, danglingCount: 0, allPaused: false))
        XCTAssertEqual(
            L.summary(selection: [a.id, c.id], endpoints: all),
            .init(selectedCount: 2, danglingCount: 0, allPaused: false))
        XCTAssertEqual(
            L.summary(selection: [b.id], endpoints: all), .init(selectedCount: 1, danglingCount: 0, allPaused: true))
        XCTAssertEqual(
            L.summary(selection: [b.id, c.id], endpoints: all),
            .init(selectedCount: 2, danglingCount: 0, allPaused: false))
        XCTAssertEqual(
            L.summary(selection: [a.id, UUID(), UUID()], endpoints: all),
            .init(selectedCount: 1, danglingCount: 2, allPaused: false))
        XCTAssertEqual(
            L.summary(selection: [UUID()], endpoints: all), .init(selectedCount: 0, danglingCount: 1, allPaused: false),
            "a deleted endpoint alone is not 'all paused'")
    }

    func testExampleCloseIsRoundedToTwoDecimalsWithoutFloatNoise() throws {
        let noisy = try XCTUnwrap(Decimal(string: "83356.00999999998"))
        let close = L.exampleClose(noisy, currency: .USD)
        XCTAssertEqual(close, 83_356.01)
        let rendered = AlertMessageRenderer.render("{{close}}", context: AlertMessageContext(close: close))
        XCTAssertEqual(rendered, "83356.01")

        XCTAssertEqual(L.exampleClose(try XCTUnwrap(Decimal(string: "1234.5")), currency: .EUR), 1_234.5)
        XCTAssertEqual(L.exampleClose(try XCTUnwrap(Decimal(string: "15234.6")), currency: .JPY), 15_235)
    }

    func testExampleCloseKeepsPrecisionForSmallPrices() throws {
        let tiny = try XCTUnwrap(Decimal(string: "0.00000278123"))
        let rendered = AlertMessageRenderer.render(
            "{{close}}", context: AlertMessageContext(close: L.exampleClose(tiny, currency: .USD)))
        XCTAssertEqual(rendered, "0.000002781", "a sub-1 price keeps four significant digits, not a flat 0.00")
    }

    func testAppendingSeparatesTokensWithOneSpace() {
        XCTAssertEqual(L.appending("{{close}}", to: ""), "{{close}}")
        XCTAssertEqual(L.appending("{{close}}", to: "BTC"), "BTC {{close}}")
        XCTAssertEqual(L.appending("{{close}}", to: "BTC "), "BTC {{close}}")
        XCTAssertEqual(L.appending("{{close}}", to: "BTC\n"), "BTC\n{{close}}")
        var text = ""
        for token in ["{{ticker}}", "{{close}}", "{{volume}}"] { text = L.appending(token, to: text) }
        XCTAssertEqual(text, "{{ticker}} {{close}} {{volume}}")
    }

    func testExamplePreviewUsesTheDefaultWhenEmptyAndKnownValuesOnly() {
        let context = AlertMessageContext(ticker: "BTCUSDT", exchange: "Binance", close: 100_000.5)
        let blank = L.examplePreview(template: "   ", context: context)
        XCTAssertEqual(blank.text, "BTCUSDT alert: 100000.5")
        XCTAssertFalse(blank.isJSON)

        let partial = L.examplePreview(template: "{{ticker}} {{volume}} on {{exchange}}", context: context)
        XCTAssertEqual(partial.text, "BTCUSDT {{volume}} on Binance", "unknown values stay literal, never invented")
    }

    func testExamplePreviewDetectsJSONAfterSubstitution() {
        let context = AlertMessageContext(ticker: "BTCUSDT", close: 100_000.5)
        let json = L.examplePreview(template: #"{"symbol":"{{ticker}}","price":{{close}}}"#, context: context)
        XCTAssertEqual(json.text, #"{"symbol":"BTCUSDT","price":100000.5}"#)
        XCTAssertTrue(json.isJSON)
        XCTAssertFalse(
            L.examplePreview(template: #"{"price":{{open}}}"#, context: context).isJSON, "unresolved → not JSON")
    }

    func testExamplePreviewNeverContainsTheSecret() {
        let context = AlertMessageContext(ticker: "BTCUSDT")
        XCTAssertEqual(L.examplePreview(template: "token {{secret}}", context: context).text, "token {{secret}}")
    }
}
