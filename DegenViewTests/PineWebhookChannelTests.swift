import XCTest

@testable import DegenView

@MainActor
final class PineWebhookChannelTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    private var database: AppDatabase!
    private var transport: RecordingWebhookTransport!
    private var bot: WebhookEndpoint!

    override func setUpWithError() throws {
        database = try AppDatabase.makeInMemory()
        transport = RecordingWebhookTransport()
        bot = WebhookEndpoint(name: "Bot", url: "https://bot.example/hook")
        try database.replaceWebhookEndpoints([bot])
    }

    private func channel(
        using transport: RecordingWebhookTransport? = nil
    ) -> WebhookPineAlertChannel {
        WebhookPineAlertChannel(
            database: database,
            service: WebhookDeliveryService(transport: transport ?? self.transport))
    }

    private func notification(
        _ message: String = "BUY", endpoints: [UUID]? = nil
    ) -> PineAlertNotification {
        PineAlertNotification(
            subscriptionID: UUID(), scriptName: "S", chartID: UUID(), symbolKey: F.dataset.symbolKey,
            timeframe: F.dataset.timeframe, barTime: Date(), message: message, frequency: .oncePerBar,
            isConfirmed: true, webhookEndpointIDs: endpoints ?? [bot.id])
    }

    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: Duration = .seconds(3)) async {
        let deadline = ContinuousClock.now + timeout
        while !condition(), ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
    }

    private func resolvedChart(_ source: String) async throws -> (chart: ChartViewModel, instanceID: UUID) {
        let chart = ChartViewModel(ticker: "BTC")
        let id = try XCTUnwrap(
            chart.addPineInstance(
                scriptID: UUID(), revisionID: UUID(), source: "//@version=6\nindicator(\"T\")\n\(source)"))
        await waitUntil(chart.pineInstanceSourceHashes[id] != nil)
        return (chart, id)
    }

    // MARK: Channel

    func testPostsTheScriptsMessageAsIs() async throws {
        let message = #"{"action":"buy","symbol":"BTCUSDT"}"#
        try await channel().deliver(notification(message))

        XCTAssertEqual(transport.requests.count, 1)
        XCTAssertEqual(transport.requests[0].bodyString, message)
        XCTAssertEqual(transport.requests[0].contentType, WebhookPayload.jsonContentType)
        XCTAssertEqual(transport.requests[0].url?.host, "bot.example")
    }

    func testNoEndpointsMeansNoRequestAndNoRows() async throws {
        try await channel().deliver(notification(endpoints: []))
        var plain = notification()
        plain.webhookEndpointIDs = nil
        try await channel().deliver(plain)

        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testDisabledAndDeletedEndpointsAreSkipped() async throws {
        var off = bot!
        off.isEnabled = false
        try database.replaceWebhookEndpoints([off])

        try await channel().deliver(notification(endpoints: [bot.id, UUID()]))

        XCTAssertTrue(transport.requests.isEmpty)
    }

    func testSameNotificationIsNeverPostedTwice() async throws {
        let value = notification()
        try await channel().deliver(value)
        try await channel().deliver(value)

        XCTAssertEqual(transport.requests.count, 1)
        XCTAssertEqual(database.webhookDeliveries(eventIDs: [value.id])[value.id]?.first?.source, .pine)
    }

    func testUnreadableEndpointTableThrowsInsteadOfPostingNothingSilently() async throws {
        try await database.writer.write { db in
            try db.execute(
                sql: "INSERT INTO webhook_endpoint (id, position, payload) VALUES (?, 5, 'garbage')",
                arguments: [UUID().uuidString])
        }

        do {
            try await channel().deliver(notification())
            XCTFail("expected the unreadable table to throw")
        } catch {}
        XCTAssertTrue(transport.requests.isEmpty)
    }

    // MARK: Channel independence

    func testAFailingWebhookDoesNotSuppressOtherChannelsOrTheEvent() async throws {
        let before = RecordingPineAlertChannel()
        let after = RecordingPineAlertChannel()
        let failing = RecordingWebhookTransport { _ in 500 }
        let dispatcher = PineAlertDispatcher(channels: [before, channel(using: failing), after])
        let value = notification()

        await dispatcher.dispatch(value)

        XCTAssertEqual(before.delivered.map(\.id), [value.id])
        XCTAssertEqual(after.delivered.map(\.id), [value.id])
        XCTAssertEqual(failing.requests.count, 1)
        XCTAssertEqual(database.webhookDeliveries(eventIDs: [value.id])[value.id]?.first?.state, .failed)
    }

    func testAThrowingChannelDoesNotSuppressTheWebhook() async throws {
        let dispatcher = PineAlertDispatcher(channels: [FailingPineAlertChannel(), channel()])
        await dispatcher.dispatch(notification())
        XCTAssertEqual(transport.requests.count, 1)
    }

    // MARK: Through the real pipeline

    /// The webhook channel adds no frequency rules: it posts exactly the alerts the other channels get.
    func testWebhookFollowsTheExistingFrequencyGuardForEveryFrequency() async throws {
        let cases: [(frequency: PineAlertFrequency, confirmed: [Bool], expected: Int)] = [
            (.all, [false, false, true], 3),
            (.oncePerBar, [false, false, true], 1),
            (.oncePerBarClose, [false, false], 0),
            (.oncePerBarClose, [false, true, true], 1),
        ]
        for (index, item) in cases.enumerated() {
            let store = PineAlertStore(database: try AppDatabase.makeInMemory())
            let recording = RecordingPineAlertChannel()
            let sent = RecordingWebhookTransport()
            let coordinator = PineAlertCoordinator(
                store: store, dispatcher: PineAlertDispatcher(channels: [recording, channel(using: sent)]))
            let (chart, instanceID) = try await resolvedChart("alert(\"x\", alert.freq_all)")
            coordinator.attach(chart)
            _ = try XCTUnwrap(
                coordinator.subscribe(
                    chart, instanceID: instanceID, scriptName: "S", note: "", webhookEndpointIDs: [bot.id]))
            let hash = try XCTUnwrap(chart.pineInstanceSourceHashes[instanceID])
            let barTime = Date()
            let barID = PineBarID(dataset: chart.pineAlertDataset, openTime: barTime)

            for confirmed in item.confirmed {
                let event = PineAlertEvent(
                    id: 1, site: 1, bar: 3, time: barTime, message: "BUY", frequency: item.frequency,
                    isRealtime: true, isConfirmed: confirmed)
                coordinator.ingest(
                    events: [event], barID: barID, chartID: chart.chartID, instanceID: instanceID, sourceHash: hash)
            }
            await waitUntil(recording.delivered.count >= item.expected && sent.requests.count >= item.expected)
            try? await Task.sleep(for: .milliseconds(80))

            XCTAssertEqual(recording.delivered.count, item.expected, "case \(index) recording")
            XCTAssertEqual(sent.requests.count, item.expected, "case \(index) webhook")
        }
    }

    func testSubscriptionWithoutWebhooksBehavesAsBefore() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let recording = RecordingPineAlertChannel()
        let coordinator = PineAlertCoordinator(
            store: store, dispatcher: PineAlertDispatcher(channels: [recording, channel()]))
        let (chart, instanceID) = try await resolvedChart("alert(\"x\", alert.freq_all)")
        coordinator.attach(chart)
        _ = try XCTUnwrap(coordinator.subscribe(chart, instanceID: instanceID, scriptName: "S", note: ""))
        let event = PineAlertEvent(
            id: 1, site: 1, bar: 3, time: Date(), message: "x", frequency: .all, isRealtime: true, isConfirmed: false)

        coordinator.ingest(
            events: [event], barID: PineBarID(dataset: chart.pineAlertDataset, openTime: event.time),
            chartID: chart.chartID, instanceID: instanceID, sourceHash: chart.pineInstanceSourceHashes[instanceID])
        await waitUntil(recording.delivered.count == 1)
        try? await Task.sleep(for: .milliseconds(80))

        XCTAssertEqual(recording.delivered.count, 1)
        XCTAssertNil(recording.delivered[0].webhookEndpointIDs)
        XCTAssertTrue(transport.requests.isEmpty)
    }

    /// Loading a script over years of candles must never replay webhooks, even when the history
    /// holds thousands of alert() calls.
    func testHistoricalRebuildSendsZeroWebhooks() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let coordinator = PineAlertCoordinator(
            store: store, dispatcher: PineAlertDispatcher(channels: [channel()]))
        let (chart, instanceID) = try await resolvedChart("alert(\"x\", alert.freq_all)")
        coordinator.attach(chart)
        _ = try XCTUnwrap(
            coordinator.subscribe(
                chart, instanceID: instanceID, scriptName: "S", note: "", webhookEndpointIDs: [bot.id]))
        let hash = try XCTUnwrap(chart.pineInstanceSourceHashes[instanceID])

        let controller = F.controller("if close > open\n    alert(\"Green\", alert.freq_all)")
        let bars = (0..<5_000).map { F.bar($0, open: 100, close: 101, closed: true) }
        let update = try XCTUnwrap(F.update(controller.rebuild(bars: bars, live: false)))
        XCTAssertTrue(update.alerts.isEmpty, "the runtime hands the coordinator nothing for history")
        XCTAssertGreaterThan(update.output.alerts.count, 0, "the report does list the historical calls")

        // Even if every historical event were pushed at the coordinator, the router refuses them.
        coordinator.ingest(
            events: update.output.alerts, barID: update.barID, chartID: chart.chartID, instanceID: instanceID,
            sourceHash: hash)
        coordinator.ingest(
            events: update.alerts, barID: update.barID, chartID: chart.chartID, instanceID: instanceID,
            sourceHash: hash)
        try? await Task.sleep(for: .milliseconds(150))

        XCTAssertTrue(transport.requests.isEmpty)
        XCTAssertTrue(store.history.isEmpty)
    }

    // MARK: alertcondition

    func testAlertconditionDeclarationSendsNothingAndFiringRendersPlaceholders() async throws {
        let header = "indicator(\"T\")"
        let body = "alertcondition(close > open, \"Bullish\", \"{{ticker}} on {{exchange}} {{close}} @{{interval}}\")"
        let symbol = PineSymbolInfo(ticker: "BTCUSDT", tickerID: "BINANCE:BTCUSDT")
        let controller = PineExecutionController(
            program: F.program(body, header: header), dataset: F.dataset, symbol: symbol)

        let loaded = controller.rebuild(
            bars: F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)], live: true, now: F.now(during: 3))
        XCTAssertTrue(
            try XCTUnwrap(F.update(loaded)).alerts.isEmpty, "declaring or replaying a condition fires nothing")
        XCTAssertTrue(transport.requests.isEmpty)

        let fired = try XCTUnwrap(
            F.update(controller.ingest(F.stream(F.bar(3, open: 100, close: 101)))))
        XCTAssertEqual(fired.alerts.map(\.message), ["BTCUSDT on BINANCE 101 @1"])

        let value = notification(try XCTUnwrap(fired.alerts.first).message)
        try await channel().deliver(value)
        XCTAssertEqual(transport.requests.map(\.bodyString), ["BTCUSDT on BINANCE 101 @1"])
    }

    func testAlertMessageIsNotSubstitutedAgain() async throws {
        let controller = PineExecutionController(
            program: F.program("if close > open\n    alert(\"{{close}} {{ticker}}\", alert.freq_all)"),
            dataset: F.dataset, symbol: PineSymbolInfo(ticker: "BTCUSDT", tickerID: "BINANCE:BTCUSDT"))
        _ = controller.rebuild(
            bars: F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)], live: true, now: F.now(during: 3))

        let fired = try XCTUnwrap(F.update(controller.ingest(F.stream(F.bar(3, open: 100, close: 101)))))

        XCTAssertEqual(fired.alerts.map(\.message), ["{{close}} {{ticker}}"], "alert() text is the script's own")
    }

    // MARK: Persistence of old data

    func testSubscriptionAndNotificationFromBeforeWebhooksStillDecode() throws {
        let subscription = PineAlertSubscription(
            chartID: UUID(), scriptName: "S", symbolKey: "binance:BTC", timeframe: "1m", sourceHash: "h",
            webhookEndpointIDs: [bot.id])
        var json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(subscription)) as? [String: Any])
        XCTAssertNotNil(json["webhookEndpointIDs"])
        json["webhookEndpointIDs"] = nil
        let decoded = try JSONDecoder().decode(
            PineAlertSubscription.self, from: JSONSerialization.data(withJSONObject: json))
        XCTAssertEqual(decoded.webhookEndpointIDs, [])
        XCTAssertEqual(decoded.id, subscription.id)

        var oldNotification = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(notification())) as? [String: Any])
        oldNotification["webhookEndpointIDs"] = nil
        let note = try JSONDecoder().decode(
            PineAlertNotification.self, from: JSONSerialization.data(withJSONObject: oldNotification))
        XCTAssertNil(note.webhookEndpointIDs)
    }

    func testEndpointSelectionLivesOnTheSubscriptionAndSurvivesEndpointRename() throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: []))
        let subscription = PineAlertSubscription(
            chartID: UUID(), scriptName: "S", symbolKey: "binance:BTC", timeframe: "1m", sourceHash: "h")
        store.add(subscription)

        coordinator.setWebhooks([bot.id], for: subscription.id)
        XCTAssertEqual(store.subscription(id: subscription.id)?.webhookEndpointIDs, [bot.id])

        var renamed = bot!
        renamed.name = "Production Bot"
        try database.replaceWebhookEndpoints([renamed])
        XCTAssertEqual(store.subscription(id: subscription.id)?.webhookEndpointIDs, [bot.id])
        XCTAssertEqual(try database.webhookEndpoints().first?.id, bot.id)
    }
}
