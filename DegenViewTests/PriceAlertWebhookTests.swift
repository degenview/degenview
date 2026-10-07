import GRDB
import XCTest

@testable import DegenView

private actor MemoryAlertRepository: AlertSnapshotRepository {
    var stored: AlertPersistenceSnapshot?
    func load() -> AlertPersistenceSnapshot? { stored }
    func save(_ snapshot: AlertPersistenceSnapshot) { stored = snapshot }
}

final class PriceAlertWebhookTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_714_571_200)
    private let asset = PortfolioAsset(
        key: "Binance:BTCUSDT", symbol: "BTC/USDT", name: "Bitcoin", source: .binance,
        quoteCurrency: .USD, metadata: ["apiSymbol": "BTCUSDT"])
    private var directory: URL!
    private var database: AppDatabase!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("PriceWebhook-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        database = try AppDatabase(path: directory.appendingPathComponent("test.sqlite").path)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: directory)
    }

    // MARK: Fixtures

    private var candle: AlertCandleSnapshot {
        AlertCandleSnapshot(
            openTime: Date(timeIntervalSince1970: 1_714_571_100), open: 99_000, high: 101_000, low: 98_500,
            close: 100_000.5, volume: 12.5, interval: "1m")
    }

    private func endpoint(_ name: String, host: String? = nil, enabled: Bool = true) throws -> WebhookEndpoint {
        let endpoint = WebhookEndpoint(
            name: name, url: "https://\(host ?? name.lowercased()).example/hook", isEnabled: enabled)
        try database.replaceWebhookEndpoints(((try? database.webhookEndpoints()) ?? []) + [endpoint])
        return endpoint
    }

    private func alert(
        endpoints: [WebhookEndpoint] = [], extra: [UUID] = [], message: String? = nil
    ) -> PriceAlert {
        PriceAlert(
            asset: asset, condition: .crossesAbove(target: 100_000), webhookEndpointIDs: endpoints.map(\.id) + extra,
            webhookMessage: message)
    }

    private func event(
        for alert: PriceAlert, age: TimeInterval = 5, origin: AlertEventOrigin = .live,
        candle: AlertCandleSnapshot? = nil
    ) -> AlertTriggerEvent {
        AlertTriggerEvent(
            alertID: alert.id, asset: asset, observedValue: 100_000.5, target: 100_000, currency: .USD,
            timestamp: now.addingTimeInterval(-age), quoteFingerprint: "q", origin: origin,
            candle: candle ?? self.candle)
    }

    private func snapshot(
        _ alerts: [PriceAlert], _ events: [AlertTriggerEvent], deliveryEnabled: Bool = true
    ) -> AlertPersistenceSnapshot {
        var snapshot = AlertPersistenceSnapshot()
        snapshot.alerts = alerts
        snapshot.history = events
        snapshot.settings.deliveryEnabled = deliveryEnabled
        return snapshot
    }

    private func dispatcher(
        _ transport: RecordingWebhookTransport, database: AppDatabase? = nil
    ) -> PriceAlertWebhookDispatcher {
        let now = now
        return PriceAlertWebhookDispatcher(
            database: database ?? self.database,
            service: WebhookDeliveryService(transport: transport), now: { now })
    }

    private func run(_ dispatcher: PriceAlertWebhookDispatcher, _ snapshot: AlertPersistenceSnapshot) async {
        for task in await dispatcher.dispatch(snapshot: snapshot) { await task.value }
    }

    // MARK: Opt-in

    func testAlertWithoutWebhooksSendsNothingAndTouchesNoTable() async throws {
        let transport = RecordingWebhookTransport()
        let plain = alert()
        await run(dispatcher(transport), snapshot([plain], [event(for: plain)]))

        XCTAssertTrue(transport.requests.isEmpty)
        let rows = try await database.reader.read { try Int.fetchOne($0, sql: "SELECT COUNT(*) FROM webhook_delivery") }
        XCTAssertEqual(rows, 0)
    }

    func testOldAlertJSONDecodesWithNoWebhooks() throws {
        let current = alert(extra: [UUID()], message: "x")
        var json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(current)) as? [String: Any])
        json["webhookEndpointIDs"] = nil
        json["webhookMessage"] = nil
        let decoded = try JSONDecoder().decode(
            PriceAlert.self, from: JSONSerialization.data(withJSONObject: json))
        XCTAssertEqual(decoded.webhookEndpointIDs, [])
        XCTAssertNil(decoded.webhookMessage)
        XCTAssertEqual(decoded.id, current.id)
    }

    func testOldTriggerEventJSONDecodesWithoutACandle() throws {
        let source = event(for: alert())
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(source)) as? [String: Any])
        json["candle"] = nil
        let decoded = try JSONDecoder().decode(
            AlertTriggerEvent.self, from: JSONSerialization.data(withJSONObject: json))
        XCTAssertNil(decoded.candle)
        XCTAssertEqual(decoded.id, source.id)
    }

    // MARK: Delivery

    func testOneEndpointGetsOneRenderedPost() async throws {
        let bot = try endpoint("Bot")
        let transport = RecordingWebhookTransport()
        let target = alert(
            endpoints: [bot], message: "{{ticker}} on {{exchange}}: {{close}} vol {{volume}} @{{interval}}")

        await run(dispatcher(transport), snapshot([target], [event(for: target)]))

        XCTAssertEqual(transport.requests.count, 1)
        let request = transport.requests[0]
        XCTAssertEqual(request.httpMethod, "POST")
        XCTAssertEqual(request.url?.host, "bot.example")
        XCTAssertEqual(request.bodyString, "BTCUSDT on Binance: 100000.5 vol 12.5 @1")
        XCTAssertEqual(request.contentType, WebhookPayload.textContentType)
    }

    func testJSONTemplateIsSentAsJSONFromTheTriggerCandle() async throws {
        let bot = try endpoint("Bot")
        let transport = RecordingWebhookTransport()
        let target = alert(
            endpoints: [bot], message: #"{"symbol":"{{ticker}}","open":{{open}},"time":"{{time}}"}"#)

        await run(dispatcher(transport), snapshot([target], [event(for: target)]))

        XCTAssertEqual(
            transport.requests[0].bodyString, #"{"symbol":"BTCUSDT","open":99000,"time":"2024-05-01T13:45:00Z"}"#)
        XCTAssertEqual(transport.requests[0].contentType, WebhookPayload.jsonContentType)
    }

    func testDefaultMessageIsUsedWhenNoneIsSet() async throws {
        let bot = try endpoint("Bot")
        let transport = RecordingWebhookTransport()
        let target = alert(endpoints: [bot])

        await run(dispatcher(transport), snapshot([target], [event(for: target)]))

        XCTAssertEqual(transport.requests[0].bodyString, "BTCUSDT alert: 100000.5")
    }

    func testMultipleEndpointsAreAttemptedIndependently() async throws {
        let a = try endpoint("A")
        let b = try endpoint("B")
        let c = try endpoint("C")
        let transport = RecordingWebhookTransport { $0.url?.host == "b.example" ? 500 : 200 }
        let target = alert(endpoints: [a, b, c])
        let trigger = event(for: target)

        await run(dispatcher(transport), snapshot([target], [trigger]))

        XCTAssertEqual(Set(transport.requests.compactMap { $0.url?.host }), ["a.example", "b.example", "c.example"])
        let rows = database.webhookDeliveries(eventIDs: [trigger.id])[trigger.id] ?? []
        XCTAssertEqual(rows.count, 3)
        let byEndpoint = Dictionary(uniqueKeysWithValues: rows.map { ($0.endpointID, $0) })
        XCTAssertEqual(byEndpoint[a.id]?.state, .delivered)
        XCTAssertEqual(byEndpoint[b.id]?.state, .failed)
        XCTAssertEqual(byEndpoint[b.id]?.statusCode, 500)
        XCTAssertEqual(byEndpoint[b.id]?.error, .httpFailure)
        XCTAssertEqual(byEndpoint[c.id]?.state, .delivered)
        XCTAssertTrue(rows.allSatisfy { $0.source == .price })
    }

    func testDisabledAndDeletedEndpointsAreSkippedWithoutRecords() async throws {
        let off = try endpoint("Off", enabled: false)
        let on = try endpoint("On")
        let transport = RecordingWebhookTransport()
        let target = alert(endpoints: [off, on], extra: [UUID()])  // plus a deleted endpoint's id
        let trigger = event(for: target)

        await run(dispatcher(transport), snapshot([target], [trigger]))

        XCTAssertEqual(transport.requests.compactMap { $0.url?.host }, ["on.example"])
        XCTAssertEqual(database.webhookDeliveries(eventIDs: [trigger.id])[trigger.id]?.count, 1)
    }

    func testDeliveryRowsHoldNoURLOrBody() async throws {
        let bot = try endpoint("Bot", host: "secret-host")
        let target = alert(endpoints: [bot], message: "SECRET-BODY")
        await run(dispatcher(RecordingWebhookTransport()), snapshot([target], [event(for: target)]))

        let dump = try await database.reader.read { db in
            try Row.fetchAll(db, sql: "SELECT * FROM webhook_delivery").map { "\($0)" }.joined()
        }
        XCTAssertFalse(dump.contains("secret-host"))
        XCTAssertFalse(dump.contains("SECRET-BODY"))
        XCTAssertFalse(dump.contains("https://"))
    }

    // MARK: Safety rules

    func testFailureNeverChangesTheTrigger() async throws {
        let bot = try endpoint("Bot")
        let target = alert(endpoints: [bot])
        let trigger = event(for: target)
        let original = snapshot([target], [trigger])

        await run(dispatcher(RecordingWebhookTransport { _ in throw URLError(.cannotConnectToHost) }), original)

        // The dispatcher only reads the snapshot; the trigger keeps its own delivery state.
        XCTAssertEqual(original.history.first?.delivery, .pending)
        let rows = database.webhookDeliveries(eventIDs: [trigger.id])[trigger.id] ?? []
        XCTAssertEqual(rows.first?.state, .failed)
        XCTAssertEqual(rows.first?.error, .networkFailure)
    }

    func testCatchUpAndStaleTriggersNeverSend() async throws {
        let bot = try endpoint("Bot")
        let transport = RecordingWebhookTransport()
        let target = alert(endpoints: [bot])

        await run(
            dispatcher(transport),
            snapshot(
                [target],
                [
                    event(for: target, origin: .catchUp),
                    event(for: target, age: PriceAlertWebhookDispatcher.maximumEventAge + 1),
                ]))

        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testDeliveryMasterSwitchSilencesWebhooksToo() async throws {
        let bot = try endpoint("Bot")
        let transport = RecordingWebhookTransport()
        let target = alert(endpoints: [bot])

        await run(dispatcher(transport), snapshot([target], [event(for: target)], deliveryEnabled: false))

        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testWebhookStillFiresWhenMacOSNotificationsAreOff() async throws {
        let bot = try endpoint("Bot")
        let transport = RecordingWebhookTransport()
        let target = alert(endpoints: [bot])
        var state = snapshot([target], [event(for: target)])
        state.settings.macOSNotificationsEnabled = false

        await run(dispatcher(transport), state)

        XCTAssertEqual(transport.requests.count, 1)
    }

    func testSameTriggerIsSentOncePerProcessAcrossTicks() async throws {
        let bot = try endpoint("Bot")
        let transport = RecordingWebhookTransport()
        let target = alert(endpoints: [bot])
        let state = snapshot([target], [event(for: target)])
        let sender = dispatcher(transport)

        await run(sender, state)
        await run(sender, state)
        await run(sender, state)

        XCTAssertEqual(transport.requests.count, 1)
    }

    /// The app and the agent are two processes on one database file. Whichever claims a trigger
    /// first sends it; the other never does, so a handoff cannot double-send a trading webhook.
    func testTwoProcessesSendATriggerExactlyOnce() async throws {
        let bot = try endpoint("Bot")
        let target = alert(endpoints: [bot])
        let state = snapshot([target], [event(for: target)])
        let otherProcess = try AppDatabase(path: directory.appendingPathComponent("test.sqlite").path)
        let first = RecordingWebhookTransport()
        let second = RecordingWebhookTransport()

        async let a: Void = run(dispatcher(first), state)
        async let b: Void = run(dispatcher(second, database: otherProcess), state)
        _ = await (a, b)

        XCTAssertEqual(first.requests.count + second.requests.count, 1)
    }

    func testUnreadableEndpointTableMeansSkipNotNoEndpoints() async throws {
        let bot = try endpoint("Bot")
        try await database.writer.write { db in
            try db.execute(
                sql: "INSERT INTO webhook_endpoint (id, position, payload) VALUES (?, 9, 'garbage')",
                arguments: [UUID().uuidString])
        }
        let transport = RecordingWebhookTransport()
        let target = alert(endpoints: [bot])
        let trigger = event(for: target)

        await run(dispatcher(transport), snapshot([target], [trigger]))

        XCTAssertTrue(transport.requests.isEmpty)
        XCTAssertNil(database.webhookDeliveries(eventIDs: [trigger.id])[trigger.id], "nothing claimed")
    }

    // MARK: Engine

    func testEngineCarriesTheQuoteCandleOntoTheTrigger() async throws {
        let engine = await LocalPriceAlertEngine(repository: MemoryAlertRepository())
        let target = alert()
        await engine.saveAlert(target, baseline: 99_000)
        func quote(_ price: Decimal, _ sequence: Int) -> MarketQuote {
            MarketQuote(
                asset: asset, price: price, currency: .USD, sourceTimestamp: Date(), receivedAt: Date(),
                maximumAge: 60, fingerprint: "q\(sequence)", candle: candle)
        }

        _ = await engine.process(quote(99_500, 1))
        let events = await engine.process(quote(100_001, 2))

        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events.first?.candle, candle)
        XCTAssertEqual(events.first?.delivery, .pending)
    }

    // MARK: Ownership

    /// Webhooks leave only from the runtime owner. The dispatcher is reachable from the host and
    /// nowhere else, and the GUI store never sends.
    func testOnlyTheRuntimeHostReachesThePriceAlertDispatcher() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("DegenView")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
            .compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" }
        var mentions: [String] = []
        for file in files {
            let text = try String(contentsOf: file, encoding: .utf8)
            if text.contains("PriceAlertWebhookDispatcher") || text.contains("WebhookDeliveryService") {
                mentions.append(file.lastPathComponent)
            }
        }
        XCTAssertFalse(mentions.contains("AlertStore.swift"))
        let allowed: Set<String> = [
            "AlertRuntimeHost.swift", "PriceAlertWebhookDispatcher.swift", "WebhookDeliveryService.swift",
            "WebhookTransport.swift", "WebhookSettingsView.swift", "WebhookPineAlertChannel.swift",
            "WebhookEndpointPicker.swift",
        ]
        XCTAssertTrue(Set(mentions).isSubset(of: allowed), "unexpected senders: \(Set(mentions).subtracting(allowed))")
    }
}
