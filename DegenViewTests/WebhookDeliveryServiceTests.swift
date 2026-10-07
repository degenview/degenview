import XCTest

@testable import DegenView

final class WebhookDeliveryServiceTests: XCTestCase {
    private func service(
        _ transport: RecordingWebhookTransport, secrets: InMemoryWebhookSecretStore = InMemoryWebhookSecretStore()
    ) -> WebhookDeliveryService {
        WebhookDeliveryService(transport: transport, secrets: secrets)
    }

    // MARK: Request shape

    func testPlainTextIsPostedAsExactUTF8WithTextContentType() async {
        let endpoint = WebhookTestFixtures.endpoint(url: "https://example.com/hook?t=1")
        let transport = RecordingWebhookTransport()

        let result = await service(transport).deliver(message: "BUY BTCUSDT", to: endpoint)

        XCTAssertTrue(result.succeeded)
        let request = transport.requests[0]
        XCTAssertEqual(request.httpMethod, "POST")
        XCTAssertEqual(request.url?.absoluteString, "https://example.com/hook?t=1")
        XCTAssertEqual(request.contentType, "text/plain; charset=utf-8")
        XCTAssertEqual(request.httpBody, Data("BUY BTCUSDT".utf8))
        XCTAssertEqual(request.timeoutInterval, 3)
    }

    func testJSONObjectIsSentUntouchedWithJSONContentType() async {
        let endpoint = WebhookTestFixtures.endpoint()
        let transport = RecordingWebhookTransport()
        let message = #"{ "action" : "buy",  "symbol":"BTCUSDT" }"#

        _ = await service(transport).deliver(message: message, to: endpoint)

        let request = transport.requests[0]
        XCTAssertEqual(request.contentType, "application/json; charset=utf-8")
        XCTAssertEqual(request.bodyString, message, "no pretty-printing, no reserialising")
    }

    func testPayloadContentTypeDetection() {
        XCTAssertEqual(WebhookPayload(message: #"{"action":"buy"}"#).contentType, WebhookPayload.jsonContentType)
        XCTAssertEqual(WebhookPayload(message: #"["BUY","BTCUSDT"]"#).contentType, WebhookPayload.jsonContentType)
        XCTAssertEqual(WebhookPayload(message: #"{"action":"buy""#).contentType, WebhookPayload.textContentType)
        XCTAssertEqual(WebhookPayload(message: "{not json}").contentType, WebhookPayload.textContentType)
        XCTAssertEqual(WebhookPayload(message: "BUY").contentType, WebhookPayload.textContentType)
        XCTAssertEqual(WebhookPayload(message: "").contentType, WebhookPayload.textContentType)
        XCTAssertEqual(WebhookPayload(message: "123").contentType, WebhookPayload.textContentType)
    }

    func testUnicodeBodyIsUTF8() async {
        let endpoint = WebhookTestFixtures.endpoint()
        let transport = RecordingWebhookTransport()
        let message = #"{"message":"価格 🚀"}"#

        _ = await service(transport).deliver(message: message, to: endpoint)

        let request = transport.requests[0]
        XCTAssertEqual(request.contentType, WebhookPayload.jsonContentType)
        XCTAssertEqual(request.httpBody, Data(message.utf8))
        XCTAssertEqual(request.bodyString, message)
    }

    func testPlaceholdersAreResolvedBeforeContentTypeIsDecided() async {
        let template = #"{"symbol":"{{ticker}}","price":{{close}}}"#
        XCTAssertEqual(WebhookPayload(message: template).contentType, WebhookPayload.textContentType)

        let rendered = AlertMessageRenderer.render(
            template, context: AlertMessageContext(ticker: "BTCUSDT", close: 100000.5))
        XCTAssertEqual(rendered, #"{"symbol":"BTCUSDT","price":100000.5}"#)
        XCTAssertEqual(WebhookPayload(message: rendered).contentType, WebhookPayload.jsonContentType)
    }

    // MARK: Methods

    func testMethodIsConfigurablePerEndpointAndDefaultsToPost() async {
        XCTAssertEqual(WebhookTestFixtures.endpoint().method, .post)
        for method in WebhookHTTPMethod.allCases {
            let transport = RecordingWebhookTransport()
            _ = await service(transport).deliver(
                message: #"{"a":1}"#, to: WebhookTestFixtures.endpoint(method: method))
            XCTAssertEqual(transport.requests.first?.httpMethod, method.rawValue)
        }
    }

    func testPutSendsTheMessageLikePost() async {
        let transport = RecordingWebhookTransport()
        _ = await service(transport).deliver(
            message: #"{"a":1}"#, to: WebhookTestFixtures.endpoint(method: .put))

        XCTAssertEqual(transport.requests[0].bodyString, #"{"a":1}"#)
        XCTAssertEqual(transport.requests[0].contentType, WebhookPayload.jsonContentType)
    }

    // MARK: Secret and headers

    func testSecretIsEncodedIntoTheURLAndRawIntoHeaders() async throws {
        let endpoint = WebhookTestFixtures.endpoint(
            url: "https://example.com/hook?token={{secret}}",
            headers: [
                WebhookHeader(name: "X-AUTH-TOKEN", value: "{{secret}}"),
                WebhookHeader(name: "Authorization", value: "Bearer {{secret}}"),
                WebhookHeader(name: "X-Static", value: "plain"),
            ])
        let secrets = InMemoryWebhookSecretStore([endpoint.id: "a b&c/="])
        let transport = RecordingWebhookTransport()

        let result = await service(transport, secrets: secrets).deliver(message: "BUY", to: endpoint)

        XCTAssertTrue(result.succeeded)
        let request = transport.requests[0]
        XCTAssertEqual(request.url?.absoluteString, "https://example.com/hook?token=a%20b%26c%2F%3D")
        XCTAssertEqual(request.value(forHTTPHeaderField: "X-AUTH-TOKEN"), "a b&c/=")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer a b&c/=")
        XCTAssertEqual(request.value(forHTTPHeaderField: "X-Static"), "plain")
    }

    func testTheSecretNeverReachesTheBody() async throws {
        let endpoint = WebhookTestFixtures.endpoint(
            url: "https://example.com/{{secret}}", headers: [WebhookHeader(name: "X-T", value: "{{secret}}")])
        let secrets = InMemoryWebhookSecretStore([endpoint.id: "TOPSECRET"])
        let transport = RecordingWebhookTransport()

        _ = await service(transport, secrets: secrets).deliver(message: "token is {{secret}}", to: endpoint)

        XCTAssertEqual(transport.requests[0].bodyString, "token is {{secret}}", "the message is never substituted")
        XCTAssertFalse(transport.requests[0].bodyString.contains("TOPSECRET"))
    }

    func testContentTypeCannotBeOverriddenByAHeader() async throws {
        // Reserved names are rejected outright, so the request is not even built.
        let endpoint = WebhookTestFixtures.endpoint(headers: [WebhookHeader(name: "Content-Type", value: "x/y")])
        let transport = RecordingWebhookTransport()

        let result = await service(transport).deliver(message: #"{"a":1}"#, to: endpoint)

        XCTAssertEqual(result.error, .invalidHeader)
        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testMissingKeychainSecretFailsWithoutSending() async {
        let endpoint = WebhookTestFixtures.endpoint(url: "https://example.com/{{secret}}")
        let transport = RecordingWebhookTransport()

        let result = await service(transport).deliver(message: "x", to: endpoint)

        XCTAssertEqual(result.error, .secretUnavailable)
        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testSecretInTheHostNeverSends() async {
        let endpoint = WebhookTestFixtures.endpoint(url: "https://{{secret}}.example.com/h")
        let secrets = InMemoryWebhookSecretStore([endpoint.id: "evil"])
        let transport = RecordingWebhookTransport()

        let result = await service(transport, secrets: secrets).deliver(message: "x", to: endpoint)

        XCTAssertEqual(result.error, .secretInHost)
        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testEndpointWithoutASecretNeverReadsTheKeychain() async {
        final class CountingStore: WebhookSecretStore, @unchecked Sendable {
            var reads = 0
            func secret(for endpointID: UUID) -> String? {
                reads += 1
                return nil
            }
            func hasSecret(for endpointID: UUID) -> Bool { false }
            func setSecret(_ secret: String, for endpointID: UUID) throws {}
            func remove(for endpointID: UUID) {}
        }
        let counting = CountingStore()
        let transport = RecordingWebhookTransport()
        let delivery = WebhookDeliveryService(transport: transport, secrets: counting)

        _ = await delivery.deliver(message: "x", to: WebhookTestFixtures.endpoint(url: "https://example.com/h"))

        XCTAssertEqual(counting.reads, 0)
        XCTAssertEqual(transport.requests.count, 1)
    }

    func testRotatedSecretAppliesOnTheNextDelivery() async {
        let endpoint = WebhookTestFixtures.endpoint(url: "https://example.com/h?t={{secret}}")
        let secrets = InMemoryWebhookSecretStore([endpoint.id: "one"])
        let transport = RecordingWebhookTransport()
        let delivery = service(transport, secrets: secrets)

        _ = await delivery.deliver(message: "x", to: endpoint)
        try? secrets.setSecret("two", for: endpoint.id)
        _ = await delivery.deliver(message: "x", to: endpoint)

        XCTAssertEqual(transport.requests.compactMap { $0.url?.query }, ["t=one", "t=two"])
    }

    func testFailureLogLineNeverContainsTheSecret() async {
        let endpoint = WebhookTestFixtures.endpoint(url: "https://example.com/{{secret}}")
        let secrets = InMemoryWebhookSecretStore([endpoint.id: "TOPSECRET"])
        let transport = RecordingWebhookTransport { _ in 500 }

        let result = await service(transport, secrets: secrets).deliver(message: "x", to: endpoint)

        XCTAssertFalse(WebhookRedactor.logLine(result).contains("TOPSECRET"))
        XCTAssertFalse(String(describing: result).contains("TOPSECRET"))
    }

    // MARK: Status classification

    func testOnlyTwoHundredSeriesIsSuccess() async {
        let cases: [(Int, Bool)] = [
            (200, true), (201, true), (204, true), (299, true), (301, false), (400, false), (401, false),
            (404, false), (429, false), (500, false), (503, false),
        ]
        for (status, expected) in cases {
            let endpoint = WebhookTestFixtures.endpoint()
            let transport = RecordingWebhookTransport { _ in status }

            let result = await service(transport).deliver(message: "x", to: endpoint)

            XCTAssertEqual(result.succeeded, expected, "HTTP \(status)")
            XCTAssertEqual(result.statusCode, status)
            XCTAssertEqual(result.error, expected ? nil : .httpFailure, "HTTP \(status)")
        }
    }

    func testTimeoutIsTypedAndConfiguredAtThreeSeconds() async {
        XCTAssertEqual(WebhookDeliveryService.requestTimeout, 3)
        let endpoint = WebhookTestFixtures.endpoint()
        let transport = RecordingWebhookTransport { _ in throw URLError(.timedOut) }

        let result = await service(transport).deliver(message: "x", to: endpoint)

        XCTAssertEqual(result.error, .timeout)
        XCTAssertNil(result.statusCode)
        XCTAssertFalse(result.succeeded)
    }

    func testOtherTransportErrorsAreNetworkFailures() async {
        let endpoint = WebhookTestFixtures.endpoint()
        let transport = RecordingWebhookTransport { _ in throw URLError(.cannotConnectToHost) }

        let result = await service(transport).deliver(message: "x", to: endpoint)

        XCTAssertEqual(result.error, .networkFailure)
    }

    func testNoAutomaticRetryAfterFailure() async {
        let endpoint = WebhookTestFixtures.endpoint()
        let transport = RecordingWebhookTransport { _ in 500 }

        _ = await service(transport).deliver(message: "BUY", to: endpoint)

        XCTAssertEqual(transport.requests.count, 1)
    }

    // MARK: URL and secret handling

    func testInvalidStoredURLNeverReachesTheTransport() async {
        let endpoint = WebhookTestFixtures.endpoint(url: "ftp://example.com/hook")
        let transport = RecordingWebhookTransport()

        let result = await service(transport).deliver(message: "x", to: endpoint, mode: .test)

        XCTAssertEqual(result.error, .unsupportedScheme)
        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testEmptyStoredURLIsReportedNotCrashed() async {
        let transport = RecordingWebhookTransport()
        let result = await service(transport).deliver(message: "x", to: WebhookTestFixtures.endpoint(url: ""))

        XCTAssertEqual(result.error, .invalidURL)
        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testEditedURLIsUsedImmediately() async {
        var endpoint = WebhookTestFixtures.endpoint(url: "https://one.example/hook")
        let transport = RecordingWebhookTransport()
        let service = service(transport)

        _ = await service.deliver(message: "x", to: endpoint)
        endpoint.url = "https://two.example/hook"
        _ = await service.deliver(message: "x", to: endpoint)

        XCTAssertEqual(transport.requests.map { $0.url?.host }, ["one.example", "two.example"])
    }

    // MARK: Enabled state

    func testProductionSkipsDisabledButTestModeDoesNot() async {
        let endpoint = WebhookTestFixtures.endpoint(enabled: false)
        let transport = RecordingWebhookTransport()
        let service = service(transport)

        let production = await service.deliver(message: "x", to: endpoint, mode: .production)
        XCTAssertEqual(production.error, .disabledEndpoint)
        XCTAssertTrue(transport.requests.isEmpty)

        let test = await service.deliver(message: "x", to: endpoint, mode: .test)
        XCTAssertTrue(test.succeeded)
        XCTAssertEqual(transport.requests.count, 1)
    }

    func testDeliverAllLeavesDisabledEndpointsOutOfTheResults() async {
        let on = WebhookTestFixtures.endpoint("On")
        let off = WebhookTestFixtures.endpoint("Off", enabled: false)
        let transport = RecordingWebhookTransport()

        let results = await service(transport).deliverAll(message: "x", to: [on, off])

        XCTAssertEqual(results.map(\.endpointID), [on.id])
        XCTAssertEqual(transport.requests.count, 1)
    }

    // MARK: Multiple endpoints

    func testOneFailingEndpointDoesNotSuppressTheOthers() async {
        let a = WebhookTestFixtures.endpoint("A", url: "https://a.example/hook")
        let b = WebhookTestFixtures.endpoint("B", url: "https://b.example/hook")
        let c = WebhookTestFixtures.endpoint("C", url: "https://c.example/hook")
        let transport = RecordingWebhookTransport { request in request.url?.host == "b.example" ? 500 : 200 }

        let results = await service(transport).deliverAll(message: "BUY", to: [a, b, c])

        XCTAssertEqual(Set(transport.requests.compactMap { $0.url?.host }), ["a.example", "b.example", "c.example"])
        let byID = Dictionary(uniqueKeysWithValues: results.map { ($0.endpointID, $0) })
        XCTAssertTrue(byID[a.id]?.succeeded == true)
        XCTAssertEqual(byID[b.id]?.error, .httpFailure)
        XCTAssertTrue(byID[c.id]?.succeeded == true)
    }

    func testSlowEndpointDoesNotDelayAnother() async {
        let slow = WebhookTestFixtures.endpoint("Slow", url: "https://slow.example/hook")
        let fast = WebhookTestFixtures.endpoint("Fast", url: "https://fast.example/hook")
        let transport = RecordingWebhookTransport { request in
            if request.url?.host == "slow.example" { try await Task.sleep(nanoseconds: 300_000_000) }
            return 200
        }

        let started = Date()
        _ = await service(transport).deliverAll(message: "x", to: [slow, fast])

        // Concurrent: total is about one delay, not two. Generous bound for loaded CI machines.
        XCTAssertLessThan(Date().timeIntervalSince(started), 0.55)
        XCTAssertEqual(transport.requests.count, 2)
    }

    // MARK: Identity

    func testRenamingKeepsIdentityAndDelivery() async {
        var endpoint = WebhookTestFixtures.endpoint("Trading Bot")
        let id = endpoint.id
        endpoint.name = "Production Bot"
        endpoint.updatedAt = Date()
        let transport = RecordingWebhookTransport()

        let result = await service(transport).deliver(message: "x", to: endpoint)

        XCTAssertEqual(endpoint.id, id)
        XCTAssertEqual(result.endpointID, id)
        XCTAssertTrue(result.succeeded)
    }

    func testEndpointRowWithoutAURLStillDecodes() throws {
        let endpoint = WebhookTestFixtures.endpoint()
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(endpoint)) as? [String: Any])
        XCTAssertEqual(json["url"] as? String, endpoint.url)
        json["url"] = nil
        let decoded = try JSONDecoder().decode(WebhookEndpoint.self, from: JSONSerialization.data(withJSONObject: json))
        XCTAssertEqual(decoded.id, endpoint.id)
        XCTAssertEqual(decoded.url, "")
        XCTAssertEqual(decoded.method, .post, "rows saved before methods existed keep posting")

        json["method"] = "GET"
        let unknown = try JSONDecoder().decode(
            WebhookEndpoint.self, from: JSONSerialization.data(withJSONObject: json))
        XCTAssertEqual(unknown.method, .post, "an unrecognised method falls back to POST, not a dropped endpoint")
    }
}
