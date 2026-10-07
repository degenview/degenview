import XCTest

@testable import DegenView

final class WebhookHeaderPolicyTests: XCTestCase {
    func testAcceptsTokenNames() {
        for name in ["X-AUTH-TOKEN", "Authorization", "x_api.key", "X-Api-Key2", "a", "!#$%&'*+-.^_`|~", " Padded "] {
            XCTAssertNil(WebhookHeaderPolicy.validate(name: name), name)
        }
    }

    func testRejectsBadNames() {
        XCTAssertEqual(WebhookHeaderPolicy.validate(name: ""), .emptyName)
        XCTAssertEqual(WebhookHeaderPolicy.validate(name: "   "), .emptyName)
        for name in ["X Auth", "X-Auth:", "X/Auth", "X(Auth)", "Ünïcode", "a,b", "a;b", "a=b", "tab\tname", "a\nb"] {
            XCTAssertEqual(WebhookHeaderPolicy.validate(name: name), .invalidName, name)
        }
        XCTAssertEqual(
            WebhookHeaderPolicy.validate(name: String(repeating: "a", count: 65)), .nameTooLong)
        XCTAssertNil(WebhookHeaderPolicy.validate(name: String(repeating: "a", count: 64)))
    }

    func testReservedNamesAreRejectedInAnyCase() {
        for name in ["Content-Type", "content-length", "HOST", "Connection", "Transfer-Encoding", "Expect", "TE"] {
            XCTAssertEqual(WebhookHeaderPolicy.validate(name: name), .reserved, name)
        }
        XCTAssertNil(WebhookHeaderPolicy.validate(name: "Authorization"))
    }

    func testValuesAllowPrintableASCIIOnly() {
        XCTAssertNil(WebhookHeaderPolicy.validate(value: "Bearer abc.DEF-123_~+/="))
        XCTAssertNil(WebhookHeaderPolicy.validate(value: "{{secret}}"))
        XCTAssertNil(WebhookHeaderPolicy.validate(value: ""))
        for value in ["a\r\nX-Injected: 1", "a\nb", "a\rb", "caf\u{e9}", "emoji 🚀", "nul\u{0}", "del\u{7f}"] {
            XCTAssertEqual(WebhookHeaderPolicy.validate(value: value), .invalidValue, value.debugDescription)
        }
        XCTAssertEqual(
            WebhookHeaderPolicy.validate(value: String(repeating: "a", count: 2049)), .valueTooLong)
    }

    func testRowErrorsFlagDuplicatesCaseInsensitivelyAndKeepTheFirst() {
        let a = WebhookHeader(name: "X-Token", value: "1")
        let b = WebhookHeader(name: "x-token", value: "2")
        let c = WebhookHeader(name: "Bad Name", value: "3")
        let errors = WebhookHeaderPolicy.errors(in: [a, b, c])
        XCTAssertNil(errors[a.id])
        XCTAssertEqual(errors[b.id], .duplicate)
        XCTAssertEqual(errors[c.id], .invalidName)
        XCTAssertFalse(WebhookHeaderPolicy.isValid([a, b, c]))
        XCTAssertTrue(WebhookHeaderPolicy.isValid([a]))
        XCTAssertTrue(WebhookHeaderPolicy.isValid([]))
    }

    func testHeaderCountIsCapped() {
        let ok = (0..<WebhookHeaderPolicy.maximumHeaders).map { WebhookHeader(name: "X-\($0)", value: "v") }
        XCTAssertTrue(WebhookHeaderPolicy.isValid(ok))
        XCTAssertFalse(WebhookHeaderPolicy.isValid(ok + [WebhookHeader(name: "X-Extra", value: "v")]))
    }

    func testSecretRules() {
        XCTAssertNil(WebhookHeaderPolicy.validate(secret: "abc123"))
        XCTAssertNil(WebhookHeaderPolicy.validate(secret: "a b&c/\u{fc}🚀"), "any printable unicode is a fine secret")
        XCTAssertEqual(WebhookHeaderPolicy.validate(secret: ""), .empty)
        XCTAssertEqual(WebhookHeaderPolicy.validate(secret: "a\nb"), .controlCharacters)
        XCTAssertEqual(WebhookHeaderPolicy.validate(secret: "a\u{0}b"), .controlCharacters)
        XCTAssertEqual(WebhookHeaderPolicy.validate(secret: String(repeating: "a", count: 2049)), .tooLong)
    }

    func testSensitiveNameDetection() {
        for name in ["Authorization", "X-API-Key", "X-Auth-Token", "api_key", "X-Secret", "Proxy-Authorization"] {
            XCTAssertTrue(WebhookHeaderPolicy.looksSensitive(name: name), name)
        }
        for name in ["Accept", "User-Agent", "X-Request-Id", "X-Source"] {
            XCTAssertFalse(WebhookHeaderPolicy.looksSensitive(name: name), name)
        }
    }

    func testEveryErrorHasAMessageThatDoesNotEchoInput() {
        let errors: [WebhookHeaderError] = [
            .emptyName, .invalidName, .nameTooLong, .reserved, .duplicate, .invalidValue, .valueTooLong,
        ]
        for error in errors { XCTAssertFalse(WebhookHeaderPolicy.message(for: error).isEmpty) }
        for error in [WebhookSecretError.empty, .tooLong, .controlCharacters] {
            XCTAssertFalse(WebhookHeaderPolicy.message(for: error).isEmpty)
        }
    }
}
