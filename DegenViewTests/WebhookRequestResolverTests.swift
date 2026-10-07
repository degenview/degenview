import XCTest

@testable import DegenView

final class WebhookRequestResolverTests: XCTestCase {
    private func resolve(
        _ url: String, headers: [WebhookHeader] = [], secret: String? = "s3cret"
    ) -> Result<WebhookResolvedRequest, WebhookDeliveryError> {
        WebhookRequestResolver.resolve(url: url, headers: headers, secret: secret)
    }

    private func url(_ result: Result<WebhookResolvedRequest, WebhookDeliveryError>) -> String? {
        if case .success(let request) = result { return request.url.absoluteString }
        return nil
    }

    private func error(_ result: Result<WebhookResolvedRequest, WebhookDeliveryError>) -> WebhookDeliveryError? {
        if case .failure(let error) = result { return error }
        return nil
    }

    // MARK: Placeholder

    func testPlaceholderToleratesSpacingAndCase() {
        for text in ["{{secret}}", "{{ secret }}", "{{Secret}}", "{{ SECRET }}", "{{\tsecret\t}}"] {
            XCTAssertTrue(WebhookSecretPlaceholder.contains(text), text)
            XCTAssertEqual(WebhookSecretPlaceholder.replacing(in: "a\(text)b", with: "X"), "aXb", text)
        }
        for text in ["{secret}", "{{secrets}}", "{{my secret}}", "$secret", "{{secret}", "secret}}"] {
            XCTAssertFalse(WebhookSecretPlaceholder.contains(text), text)
        }
    }

    func testReplacementIsLiteral() {
        XCTAssertEqual(WebhookSecretPlaceholder.replacing(in: "{{secret}}-{{secret}}", with: "$1\\0&"), "$1\\0&-$1\\0&")
    }

    // MARK: URL encoding

    func testSecretInQueryIsPercentEncoded() {
        XCTAssertEqual(
            url(resolve("https://example.com/webhook?token={{secret}}", secret: "a b&c/\u{fc}")),
            "https://example.com/webhook?token=a%20b%26c%2F%C3%BC")
    }

    func testEveryReservedCharacterIsEscapedAndUnreservedIsNot() {
        XCTAssertEqual(WebhookRequestResolver.urlEncoded("a b/c?d&e=f#g+h%i"), "a%20b%2Fc%3Fd%26e%3Df%23g%2Bh%25i")
        XCTAssertEqual(WebhookRequestResolver.urlEncoded("AZaz09-._~"), "AZaz09-._~")
        XCTAssertEqual(WebhookRequestResolver.urlEncoded("\u{e9}🚀"), "%C3%A9%F0%9F%9A%80")
        XCTAssertEqual(WebhookRequestResolver.urlEncoded("@:!$'()*,;"), "%40%3A%21%24%27%28%29%2A%2C%3B")
    }

    func testSecretInPathSegmentAndFragmentAndBoth() {
        XCTAssertEqual(
            url(resolve("https://example.com/hook/{{secret}}/run", secret: "a/b")),
            "https://example.com/hook/a%2Fb/run")
        XCTAssertEqual(
            url(resolve("https://example.com/h?a={{secret}}&b={{ secret }}#{{secret}}", secret: "x y")),
            "https://example.com/h?a=x%20y&b=x%20y#x%20y")
    }

    func testPlainURLWithoutSecretNeedsNone() {
        XCTAssertEqual(url(resolve("https://example.com/h?x=1", secret: nil)), "https://example.com/h?x=1")
        XCTAssertEqual(url(resolve("https://example.com/h", secret: "unused")), "https://example.com/h")
    }

    // MARK: Where the secret may go

    func testSecretIsRejectedInTheAuthority() {
        for template in [
            "https://{{secret}}.example.com/h", "https://example.{{secret}}/h", "https://user:{{secret}}@example.com/h",
            "https://example.com:{{secret}}/h", "{{secret}}://example.com/h", "https://{{secret}}",
            "https://{{secret}}?a=1",
        ] {
            XCTAssertEqual(error(resolve(template)), .secretInHost, template)
        }
        XCTAssertNil(error(resolve("https://example.com/{{secret}}")))
        XCTAssertNil(error(resolve("https://example.com?t={{secret}}")))
    }

    func testMissingOrEmptySecretFailsOnlyWhenUsed() {
        XCTAssertEqual(error(resolve("https://example.com/{{secret}}", secret: nil)), .secretUnavailable)
        XCTAssertEqual(error(resolve("https://example.com/{{secret}}", secret: "")), .secretUnavailable)
        let header = WebhookHeader(name: "X-Token", value: "{{secret}}")
        XCTAssertEqual(error(resolve("https://example.com", headers: [header], secret: nil)), .secretUnavailable)
        XCTAssertNil(error(resolve("https://example.com", secret: nil)))
    }

    func testTemplateStillGoesThroughTheURLPolicy() {
        XCTAssertEqual(error(resolve("ftp://example.com/{{secret}}")), .unsupportedScheme)
        XCTAssertEqual(error(resolve("not a url {{secret}}")), .invalidURL)
        XCTAssertEqual(error(resolve("")), .invalidURL)
    }

    // MARK: Headers

    func testHeaderGetsTheRawSecret() throws {
        let headers = [
            WebhookHeader(name: "X-AUTH-TOKEN", value: "{{secret}}"),
            WebhookHeader(name: " Authorization ", value: " Bearer {{ secret }} "),
        ]
        let request = try XCTUnwrap(try? resolve("https://example.com", headers: headers, secret: "a b&c/=+").get())
        XCTAssertEqual(request.headers.map(\.name), ["X-AUTH-TOKEN", "Authorization"])
        XCTAssertEqual(request.headers.map(\.value), ["a b&c/=+", "Bearer a b&c/=+"], "no URL encoding in headers")
    }

    func testSecretThatIsNotHeaderSafeIsRejectedOnlyInAHeader() {
        let header = WebhookHeader(name: "X-Token", value: "{{secret}}")
        XCTAssertEqual(error(resolve("https://example.com", headers: [header], secret: "caf\u{e9}")), .invalidHeader)
        XCTAssertEqual(
            error(resolve("https://example.com", headers: [header], secret: "a\r\nX-Evil: 1")), .invalidHeader)
        XCTAssertEqual(
            url(resolve("https://example.com/{{secret}}", secret: "caf\u{e9}")), "https://example.com/caf%C3%A9",
            "the same secret is fine in a URL")
    }

    func testInvalidHeadersFailBeforeAnythingIsSent() {
        XCTAssertEqual(
            error(resolve("https://example.com", headers: [WebhookHeader(name: "Bad Name", value: "x")])),
            .invalidHeader)
        XCTAssertEqual(
            error(resolve("https://example.com", headers: [WebhookHeader(name: "Content-Type", value: "x")])),
            .invalidHeader)
        XCTAssertEqual(
            error(
                resolve(
                    "https://example.com",
                    headers: [WebhookHeader(name: "A", value: "1"), WebhookHeader(name: "a", value: "2")])),
            .invalidHeader)
    }

    // MARK: Previews and detection

    func testMaskedPreviewNeverContainsTheSecret() {
        XCTAssertEqual(
            WebhookRequestResolver.maskedURL("https://example.com/hook?token={{ secret }}"),
            "https://example.com/hook?token=••••••")
    }

    func testReferencesDetectsUseInURLOrHeaders() {
        XCTAssertTrue(WebhookRequestResolver.references(url: "https://e.com/{{secret}}", headers: []))
        XCTAssertTrue(
            WebhookRequestResolver.references(
                url: "https://e.com", headers: [WebhookHeader(name: "A", value: "{{secret}}")]))
        XCTAssertFalse(
            WebhookRequestResolver.references(url: "https://e.com", headers: [WebhookHeader(name: "A", value: "x")]))
        XCTAssertTrue(WebhookTestFixtures.endpoint(url: "https://e.com/{{secret}}").referencesSecret)
    }
}
