import GRDB
import XCTest

@testable import DegenView

@MainActor
private final class FakeWebhookReferences: WebhookReferenceSource {
    var counts: [UUID: Int] = [:]
    var removed: [UUID] = []
    func referenceCount(to endpointID: UUID) -> Int { counts[endpointID] ?? 0 }
    func removeReferences(to endpointID: UUID) { removed.append(endpointID) }
}

@MainActor
final class WebhookEndpointStoreTests: XCTestCase {
    private var database: AppDatabase!
    private var references: FakeWebhookReferences!
    private var secrets: InMemoryWebhookSecretStore!

    override func setUpWithError() throws {
        database = try AppDatabase.makeInMemory()
        references = FakeWebhookReferences()
        secrets = InMemoryWebhookSecretStore()
    }

    private func makeStore() -> WebhookEndpointStore {
        WebhookEndpointStore(database: database, secrets: secrets, references: references)
    }

    func testEndpointSurvivesReloadWithItsURLInSQLite() throws {
        let store = makeStore()
        let endpoint = try store.add(
            name: "Trading Bot", description: "Prod", url: "https://example.com/hook/abc123secret", isEnabled: true)

        let reloaded = makeStore()
        XCTAssertEqual(reloaded.endpoints, [endpoint])
        XCTAssertEqual(reloaded.endpoint(id: endpoint.id)?.url, "https://example.com/hook/abc123secret")
        XCTAssertEqual(reloaded.address(for: endpoint.id), "https://example.com/•••")

        let payloads = try database.reader.read { try String.fetchAll($0, sql: "SELECT payload FROM webhook_endpoint") }
        XCTAssertEqual(payloads.count, 1)
        XCTAssertTrue(payloads[0].contains("abc123secret"), "saved as plain text, not hashed or encrypted")
        XCTAssertTrue(payloads[0].contains("example.com"))
    }

    func testMethodPersistsAndEditsKeepOrChangeIt() throws {
        let store = makeStore()
        let endpoint = try store.add(
            name: "Bot", description: "", url: "https://example.com/h", method: .put, isEnabled: true)
        XCTAssertEqual(makeStore().endpoint(id: endpoint.id)?.method, .put)

        try store.update(id: endpoint.id, name: "Bot", description: "", url: nil, isEnabled: true)
        XCTAssertEqual(store.endpoint(id: endpoint.id)?.method, .put, "an edit that doesn't pass a method keeps it")

        try store.update(id: endpoint.id, name: "Bot", description: "", url: nil, method: .post, isEnabled: true)
        XCTAssertEqual(makeStore().endpoint(id: endpoint.id)?.method, .post)
    }

    func testRenameKeepsIdentityAndURL() throws {
        let store = makeStore()
        let endpoint = try store.add(
            name: "Trading Bot", description: "", url: "https://example.com/h", isEnabled: true)

        try store.update(
            id: endpoint.id, name: "Production Bot", description: "x", url: nil, isEnabled: true)

        let renamed = try XCTUnwrap(store.endpoint(id: endpoint.id))
        XCTAssertEqual(renamed.id, endpoint.id)
        XCTAssertEqual(renamed.name, "Production Bot")
        XCTAssertEqual(store.endpoint(id: endpoint.id)?.url, "https://example.com/h")
        XCTAssertEqual(makeStore().endpoints.first?.name, "Production Bot")
    }

    func testEditingTheURLReplacesTheSecret() throws {
        let store = makeStore()
        let endpoint = try store.add(name: "Bot", description: "", url: "https://one.example/h", isEnabled: true)

        try store.update(id: endpoint.id, name: "Bot", description: "", url: "https://two.example/h", isEnabled: true)

        XCTAssertEqual(store.endpoint(id: endpoint.id)?.url, "https://two.example/h")
        XCTAssertEqual(makeStore().endpoint(id: endpoint.id)?.url, "https://two.example/h")
        XCTAssertEqual(store.address(for: endpoint.id), "https://two.example/•••")
    }

    func testDisablingKeepsTheEndpointAndItsReferences() throws {
        let store = makeStore()
        let endpoint = try store.add(name: "Bot", description: "", url: "https://example.com/h", isEnabled: true)

        try store.setEnabled(false, for: endpoint.id)

        XCTAssertEqual(store.endpoint(id: endpoint.id)?.isEnabled, false)
        XCTAssertEqual(makeStore().endpoint(id: endpoint.id)?.isEnabled, false)
        XCTAssertTrue(references.removed.isEmpty)
        XCTAssertEqual(store.endpoint(id: endpoint.id)?.url, "https://example.com/h")
    }

    func testDeleteRemovesRowAndReferences() throws {
        let store = makeStore()
        let endpoint = try store.add(name: "Bot", description: "", url: "https://example.com/h", isEnabled: true)
        references.counts[endpoint.id] = 4
        XCTAssertEqual(store.referenceCount(to: endpoint.id), 4)

        try store.delete(id: endpoint.id)

        XCTAssertTrue(store.endpoints.isEmpty)
        XCTAssertTrue(makeStore().endpoints.isEmpty)
        XCTAssertEqual(references.removed, [endpoint.id])
    }

    func testValidationRejectsBadInputWithoutWritingAnything() {
        let store = makeStore()
        XCTAssertThrowsError(try store.add(name: "Bot", description: "", url: "ftp://example.com", isEnabled: true)) {
            XCTAssertEqual($0 as? WebhookEndpointError, .invalidURL(.unsupportedScheme))
        }
        XCTAssertThrowsError(try store.add(name: "  ", description: "", url: "https://example.com", isEnabled: true)) {
            XCTAssertEqual($0 as? WebhookEndpointError, .emptyName)
        }
        XCTAssertTrue(store.endpoints.isEmpty)
        XCTAssertTrue((try? database.webhookEndpoints())?.isEmpty == true)
    }

    func testUnreadableTableTurnsWritesOffInsteadOfReadingAsEmpty() throws {
        try database.writer.write { db in
            try db.execute(
                sql: "INSERT INTO webhook_endpoint (id, position, payload) VALUES (?, 0, ?)",
                arguments: [UUID().uuidString, "not json"])
        }
        XCTAssertThrowsError(try database.webhookEndpoints())

        let store = makeStore()

        XCTAssertTrue(store.loadFailed)
        XCTAssertThrowsError(try store.add(name: "Bot", description: "", url: "https://example.com", isEnabled: true)) {
            XCTAssertEqual($0 as? WebhookEndpointError, .persistenceUnavailable)
        }
        // The bad row is still there: nothing overwrote it.
        let count = try database.reader.read { try Int.fetchOne($0, sql: "SELECT COUNT(*) FROM webhook_endpoint") }
        XCTAssertEqual(count, 1)
    }

    // MARK: Secret and headers

    func testSecretAndHeadersRoundTripAndTheTemplateIsStoredVerbatim() throws {
        let store = makeStore()
        let headers = [WebhookHeader(name: " X-AUTH-TOKEN ", value: " {{secret}} ")]
        let endpoint = try store.add(
            name: "Bot", description: "", url: " https://example.com/hook?token={{secret}} ", headers: headers,
            secret: "TOPSECRET-123", isEnabled: true)

        XCTAssertEqual(endpoint.url, "https://example.com/hook?token={{secret}}", "braces are not percent-encoded away")
        XCTAssertEqual(endpoint.headers.map(\.name), ["X-AUTH-TOKEN"])
        XCTAssertEqual(endpoint.headers.map(\.value), ["{{secret}}"])
        XCTAssertTrue(endpoint.referencesSecret)

        let reloaded = makeStore()
        XCTAssertEqual(reloaded.endpoint(id: endpoint.id)?.headers, endpoint.headers)
        XCTAssertTrue(reloaded.hasSecret(endpoint.id))
        XCTAssertEqual(reloaded.secret(for: endpoint.id), "TOPSECRET-123")
    }

    func testSecretValueIsNeverInTheDatabase() async throws {
        let store = makeStore()
        let endpoint = try store.add(
            name: "Bot", description: "", url: "https://example.com/{{secret}}", secret: "TOPSECRET-123",
            isEnabled: true)
        XCTAssertNotNil(endpoint)

        let dump = try await database.reader.read { db in
            try Row.fetchAll(db, sql: "SELECT * FROM webhook_endpoint").map { "\($0)" }.joined()
        }
        XCTAssertFalse(dump.contains("TOPSECRET"))
        XCTAssertTrue(dump.contains("secret"), "only the placeholder is saved")
    }

    func testUpdateKeepsChangesOrRemovesTheSecret() throws {
        let store = makeStore()
        let endpoint = try store.add(
            name: "Bot", description: "", url: "https://example.com/{{secret}}", secret: "one", isEnabled: true)

        try store.update(id: endpoint.id, name: "Bot 2", description: "", url: nil, isEnabled: true)
        XCTAssertEqual(store.secret(for: endpoint.id), "one", "nil keeps")

        try store.update(id: endpoint.id, name: "Bot 2", description: "", url: nil, secret: "two", isEnabled: true)
        XCTAssertEqual(store.secret(for: endpoint.id), "two")

        XCTAssertThrowsError(
            try store.update(id: endpoint.id, name: "Bot 2", description: "", url: nil, secret: "", isEnabled: true)
        ) { XCTAssertEqual($0 as? WebhookEndpointError, .secretRequired) }
        XCTAssertEqual(store.secret(for: endpoint.id), "two", "a rejected edit changes nothing")

        try store.update(
            id: endpoint.id, name: "Bot 2", description: "", url: "https://example.com/plain", secret: "",
            isEnabled: true)
        XCTAssertNil(store.secret(for: endpoint.id))
        XCTAssertFalse(store.hasSecret(endpoint.id))
    }

    func testTemplateUsingTheSecretNeedsOne() {
        let store = makeStore()
        XCTAssertThrowsError(
            try store.add(name: "Bot", description: "", url: "https://example.com/{{secret}}", isEnabled: true)
        ) { XCTAssertEqual($0 as? WebhookEndpointError, .secretRequired) }
        XCTAssertThrowsError(
            try store.add(
                name: "Bot", description: "", url: "https://example.com",
                headers: [WebhookHeader(name: "X-T", value: "{{secret}}")], isEnabled: true)
        ) { XCTAssertEqual($0 as? WebhookEndpointError, .secretRequired) }
        XCTAssertTrue(store.endpoints.isEmpty)
    }

    func testInvalidInputsAreRejectedBeforeAnythingIsWritten() {
        let store = makeStore()
        func add(url: String = "https://example.com", headers: [WebhookHeader] = [], secret: String = "") throws {
            try store.add(name: "Bot", description: "", url: url, headers: headers, secret: secret, isEnabled: true)
        }
        XCTAssertThrowsError(try add(url: "https://{{secret}}.example.com", secret: "x")) {
            XCTAssertEqual($0 as? WebhookEndpointError, .invalidURL(.secretInHost))
        }
        XCTAssertThrowsError(try add(headers: [WebhookHeader(name: "Bad Name", value: "x")])) {
            XCTAssertEqual($0 as? WebhookEndpointError, .invalidHeaders)
        }
        XCTAssertThrowsError(try add(headers: [WebhookHeader(name: "Content-Type", value: "x")])) {
            XCTAssertEqual($0 as? WebhookEndpointError, .invalidHeaders)
        }
        XCTAssertThrowsError(try add(secret: "line\nbreak")) {
            XCTAssertEqual($0 as? WebhookEndpointError, .invalidSecret(.controlCharacters))
        }
        XCTAssertTrue(store.endpoints.isEmpty)
        XCTAssertFalse(store.hasSecret(UUID()))
    }

    func testKeychainFailureSavesNothing() {
        secrets.failsToWrite = true
        let store = makeStore()
        XCTAssertThrowsError(
            try store.add(
                name: "Bot", description: "", url: "https://example.com/{{secret}}", secret: "x", isEnabled: true)
        ) { XCTAssertEqual($0 as? WebhookEndpointError, .secretSaveFailed) }
        XCTAssertTrue(store.endpoints.isEmpty)
        XCTAssertTrue((try? database.webhookEndpoints())?.isEmpty == true)
    }

    func testDatabaseFailureRollsTheKeychainBack() throws {
        let store = makeStore()
        let endpoint = try store.add(
            name: "Bot", description: "", url: "https://example.com/{{secret}}", secret: "original", isEnabled: true)
        try database.writer.write { try $0.execute(sql: "DROP TABLE webhook_endpoint") }

        XCTAssertThrowsError(
            try store.update(
                id: endpoint.id, name: "Bot", description: "", url: nil, secret: "changed", isEnabled: true)
        ) { XCTAssertEqual($0 as? WebhookEndpointError, .saveFailed) }

        XCTAssertEqual(secrets.secret(for: endpoint.id), "original", "the Keychain was restored")
        XCTAssertEqual(store.endpoint(id: endpoint.id)?.name, "Bot")

        XCTAssertThrowsError(
            try store.add(
                name: "New", description: "", url: "https://example.com/{{secret}}", secret: "x", isEnabled: true)
        ) { XCTAssertEqual($0 as? WebhookEndpointError, .saveFailed) }
        XCTAssertEqual(store.endpoints.count, 1)
        XCTAssertEqual(secrets.hasSecret(for: UUID()), false)
    }

    func testDeleteRemovesTheKeychainItem() throws {
        let store = makeStore()
        let endpoint = try store.add(
            name: "Bot", description: "", url: "https://example.com/{{secret}}", secret: "x", isEnabled: true)
        XCTAssertTrue(store.hasSecret(endpoint.id))

        try store.delete(id: endpoint.id)

        XCTAssertNil(secrets.secret(for: endpoint.id))
        XCTAssertFalse(store.hasSecret(endpoint.id))
    }

    func testRowsFromBeforeHeadersExistedStillLoad() throws {
        try database.writer.write { db in
            try db.execute(
                sql: "INSERT INTO webhook_endpoint (id, position, payload) VALUES (?, 0, ?)",
                arguments: [
                    UUID().uuidString,
                    #"{"id":"\#(UUID().uuidString)","name":"Old","description":"","url":"https://e.com","isEnabled":true,"createdAt":0,"updatedAt":0}"#,
                ])
        }
        let store = makeStore()
        XCTAssertFalse(store.loadFailed)
        XCTAssertEqual(store.endpoints.first?.headers, [])
        XCTAssertEqual(store.endpoints.first?.method, .post)
    }

    func testDefaultTestMessageIsJSON() {
        XCTAssertEqual(WebhookSettingsView.defaultTestPayload, #"{"event":"DegenView webhook test"}"#)
        XCTAssertEqual(
            WebhookPayload(message: WebhookSettingsView.defaultTestPayload).contentType, WebhookPayload.jsonContentType)
    }

    func testTestStateWording() {
        let id = UUID()
        func result(_ error: WebhookDeliveryError?, status: Int?, duration: TimeInterval) -> WebhookTestState {
            WebhookTestState(
                WebhookDeliveryResult(
                    endpointID: id, timestamp: Date(), duration: duration, statusCode: status, error: error))
        }
        let ok = result(nil, status: 200, duration: 0.143)
        XCTAssertEqual(ok.title, "Delivered")
        XCTAssertEqual(ok.detail, "HTTP 200 · 143 ms")
        let denied = result(.httpFailure, status: 401, duration: 0.098)
        XCTAssertEqual(denied.title, "Failed")
        XCTAssertEqual(denied.detail, "HTTP 401 · 98 ms")
        let slow = result(.timeout, status: nil, duration: 3.0)
        XCTAssertEqual(slow.title, "Timed out")
        XCTAssertEqual(slow.detail, "3.0 s")
        XCTAssertEqual(WebhookTestState.notTested.title, "Not tested")
        XCTAssertEqual(WebhookTestState.testing.title, "Testing…")
    }
}
