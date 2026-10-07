import XCTest

@testable import DegenView

final class WebhookURLPolicyTests: XCTestCase {
    private func error(_ string: String) -> WebhookDeliveryError? {
        if case .failure(let error) = WebhookURLPolicy.validate(string) { return error }
        return nil
    }

    func testAcceptsHTTPAndHTTPSWithDefaultAndExplicitStandardPorts() {
        for string in [
            "https://example.com/hook", "http://example.com/hook", "https://example.com:443/hook",
            "http://example.com:80/hook", "HTTPS://Example.com/Hook?token=abc#frag",
        ] {
            XCTAssertNil(error(string), string)
        }
    }

    func testLocalAndPrivateDestinationsAreAllowedOnAnyPort() {
        for string in [
            "http://localhost:8080/hook", "http://127.0.0.1:5000", "http://[::1]:9000/x", "http://192.168.1.20:3000/h",
            "http://10.0.0.5/h", "http://169.254.1.1:81/h", "https://bridge.local:8443/h",
        ] {
            XCTAssertNil(error(string), string)
        }
    }

    func testRejectsUnsupportedSchemes() {
        for string in [
            "ftp://example.com", "file:///tmp/foo", "javascript:alert(1)", "ws://example.com", "mailto:a@b.c",
        ] {
            XCTAssertEqual(error(string), .unsupportedScheme, string)
        }
    }

    func testRejectsMalformedAndHostlessURLs() {
        for string in ["", "   ", "example.com/hook", "https://", "http:///path", "not a url", "https://exa mple.com"] {
            XCTAssertEqual(error(string), .invalidURL, string)
        }
    }

    func testRejectsOutOfRangePorts() {
        XCTAssertEqual(error("https://example.com:0/hook"), .unsupportedPort)
        XCTAssertNotNil(error("https://example.com:70000/hook"))
    }

    func testSurroundingWhitespaceIsTrimmed() {
        guard case .success(let url) = WebhookURLPolicy.validate("  https://example.com/hook \n") else {
            return XCTFail("expected a valid URL")
        }
        XCTAssertEqual(url.absoluteString, "https://example.com/hook")
    }

    func testEditorMessagesNeverEchoTheURL() {
        for error in [WebhookDeliveryError.invalidURL, .unsupportedScheme, .unsupportedPort] {
            XCTAssertFalse(WebhookURLPolicy.message(for: error).contains("http://secret"))
        }
    }
}

final class WebhookRedactorTests: XCTestCase {
    func testKeepsOnlySchemeHostAndPort() {
        XCTAssertEqual(WebhookRedactor.redact("https://example.com/webhook/abc123secret"), "https://example.com/•••")
        XCTAssertEqual(
            WebhookRedactor.redact("http://localhost:8080/h?token=abc123secret"), "http://localhost:8080/•••")
        XCTAssertEqual(WebhookRedactor.redact("https://user:pass@example.com/x"), "https://example.com/•••")
    }

    func testSecretNeverAppearsInRedactionOrLogLine() {
        let secret = "abc123secret"
        let url = "https://example.com/webhook/\(secret)?key=\(secret)"
        XCTAssertFalse(WebhookRedactor.redact(url).contains(secret))
        XCTAssertFalse(WebhookRedactor.redact(URL(string: url)!).contains(secret))

        let result = WebhookDeliveryResult(
            endpointID: UUID(), timestamp: Date(), duration: 0.12, statusCode: 401, error: .httpFailure)
        let line = WebhookRedactor.logLine(result)
        XCTAssertTrue(line.contains("status=401"))
        XCTAssertTrue(line.contains("endpointID="))
        XCTAssertFalse(line.contains("://"))
    }

    func testGarbageRedactsToAMarker() {
        XCTAssertEqual(WebhookRedactor.redact("garbage"), "•••")
    }
}
