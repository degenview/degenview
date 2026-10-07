import XCTest

@testable import DegenView

final class WebhookSecretPlacementTests: XCTestCase {
    private typealias P = WebhookSecretPlacement

    func testAddsTheParameterWithTheRightSeparator() {
        XCTAssertEqual(P.addingParameter("token", to: "https://e.com/hook"), "https://e.com/hook?token={{secret}}")
        XCTAssertEqual(
            P.addingParameter("token", to: "https://e.com/hook?a=1"), "https://e.com/hook?a=1&token={{secret}}")
        XCTAssertEqual(P.addingParameter("token", to: "https://e.com/hook?"), "https://e.com/hook?token={{secret}}")
        XCTAssertEqual(
            P.addingParameter("token", to: "https://e.com/hook?a=1&"), "https://e.com/hook?a=1&token={{secret}}")
        XCTAssertEqual(
            P.addingParameter("key", to: "https://e.com/hook#frag"), "https://e.com/hook?key={{secret}}#frag")
        XCTAssertEqual(
            P.addingParameter("key", to: "https://e.com/h?a=1#frag"), "https://e.com/h?a=1&key={{secret}}#frag")
    }

    func testParameterIsIdempotentAndIgnoresEmptyURLs() {
        let once = P.addingParameter("token", to: "https://e.com/hook")
        XCTAssertEqual(P.addingParameter("token", to: once), once)
        XCTAssertEqual(P.addingParameter("token", to: ""), "")
        XCTAssertEqual(P.addingParameter("token", to: "https://e.com/{{secret}}"), "https://e.com/{{secret}}")
    }

    func testBearerAndAPIKeyHeaders() {
        let bearer = P.apply(.bearerToken, url: "u", headers: [])
        XCTAssertEqual(bearer.headers.map(\.name), ["Authorization"])
        XCTAssertEqual(bearer.headers.map(\.value), ["Bearer {{secret}}"])
        XCTAssertTrue(bearer.touchesHeaders)

        let key = P.apply(.apiKey, url: "u", headers: bearer.headers)
        XCTAssertEqual(key.headers.map(\.name), ["Authorization", "X-API-Key"])
        XCTAssertEqual(key.headers.last?.value, "{{secret}}")
    }

    func testHeaderPresetsAreIdempotentAndReplaceALiteralValue() {
        let first = P.apply(.bearerToken, url: "u", headers: []).headers
        XCTAssertEqual(P.apply(.bearerToken, url: "u", headers: first).headers, first)

        let literal = [WebhookHeader(name: "authorization", value: "Bearer old")]
        let replaced = P.apply(.bearerToken, url: "u", headers: literal).headers
        XCTAssertEqual(replaced.count, 1)
        XCTAssertEqual(replaced[0].value, "Bearer {{secret}}")
        XCTAssertEqual(replaced[0].id, literal[0].id)
    }

    func testCustomHeaderAddsOneBlankNamedRowOnce() {
        let first = P.apply(.customHeader, url: "u", headers: [])
        XCTAssertEqual(first.headers.count, 1)
        XCTAssertEqual(first.headers[0].name, "")
        XCTAssertEqual(first.headers[0].value, "{{secret}}")
        XCTAssertEqual(P.apply(.customHeader, url: "u", headers: first.headers).headers.count, 1)
    }

    func testURLChoiceDoesNotTouchHeaders() {
        let edit = P.apply(.urlParameter("token"), url: "https://e.com", headers: [])
        XCTAssertEqual(edit.url, "https://e.com?token={{secret}}")
        XCTAssertFalse(edit.touchesHeaders)
    }

    func testCredentialNudgeFindsOnlyPlainCredentials() {
        let plain = WebhookHeader(name: "X-API-Key", value: "abc123")
        let safe = WebhookHeader(name: "Authorization", value: "Bearer {{secret}}")
        let benign = WebhookHeader(name: "Accept", value: "json")
        let blank = WebhookHeader(name: "X-Token", value: "  ")
        XCTAssertEqual(P.plainCredentialHeader(in: [safe, benign, blank, plain])?.id, plain.id)
        XCTAssertNil(P.plainCredentialHeader(in: [safe, benign, blank]))
    }

    func testMovingACredentialToTheSecretKeepsTheScheme() {
        XCTAssertEqual(
            P.movingToSecret(WebhookHeader(name: "Authorization", value: "Bearer abc 123")).secret, "abc 123")
        XCTAssertEqual(
            P.movingToSecret(WebhookHeader(name: "Authorization", value: "bearer abc")).value, "Bearer {{secret}}")
        let whole = P.movingToSecret(WebhookHeader(name: "X-API-Key", value: " key-1 "))
        XCTAssertEqual(whole.secret, "key-1")
        XCTAssertEqual(whole.value, "{{secret}}")
    }
}
