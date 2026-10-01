import XCTest

@testable import DegenView

@MainActor
final class PineAlertRoutingTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    private let chartID = UUID()
    private let sourceHash = "hash-a"

    private func subscription(
        chartID: UUID? = nil, symbolKey: String = F.dataset.symbolKey, timeframe: String = F.dataset.timeframe,
        sourceHash: String? = nil, state: PineAlertSubscription.State = .active,
        createdAt: Date = Date(timeIntervalSince1970: 0)
    ) -> PineAlertSubscription {
        PineAlertSubscription(
            chartID: chartID ?? self.chartID, scriptName: "Script", symbolKey: symbolKey, timeframe: timeframe,
            sourceHash: sourceHash ?? self.sourceHash, state: state, createdAt: createdAt)
    }

    private func route(
        _ update: PineExecutionUpdate, _ subscriptions: [PineAlertSubscription],
        guard frequencyGuard: inout PineAlertFrequencyGuard, sourceHash: String? = "hash-a"
    ) -> [PineAlertRouter.Routed] {
        PineAlertRouter.route(
            events: update.alerts, barID: update.barID, chartID: chartID, sourceHash: sourceHash,
            subscriptions: subscriptions, guard: &frequencyGuard)
    }

    private func ticks(
        _ controller: PineExecutionController, bar index: Int, open: Double, closes: [Double],
        closingLast: Bool = true
    ) -> [PineExecutionUpdate] {
        closes.enumerated().compactMap { offset, close in
            F.update(
                controller.ingest(
                    F.stream(
                        F.bar(index, open: open, close: close, closed: closingLast && offset == closes.count - 1))))
        }
    }

    /// Loads three flat bars plus a forming red one, then feeds 101, 102 and a closing 103, then a new bar.
    private func deliveries(_ frequency: String) throws -> [Int] {
        let controller = F.controller("if close > open\n    alert(\"Up\", alert.freq_\(frequency))")
        let loaded = controller.rebuild(
            bars: F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)], live: true, now: F.now(during: 3))
        var frequencyGuard = PineAlertFrequencyGuard()
        let subscriptions = [subscription()]
        XCTAssertEqual(route(try XCTUnwrap(F.update(loaded)), subscriptions, guard: &frequencyGuard).count, 0)
        var counts = ticks(controller, bar: 3, open: 100, closes: [101, 102, 103]).map {
            route($0, subscriptions, guard: &frequencyGuard).count
        }
        counts += ticks(controller, bar: 4, open: 103, closes: [105], closingLast: false).map {
            route($0, subscriptions, guard: &frequencyGuard).count
        }
        return counts
    }

    // MARK: - Historical and live

    func testLoadingHistoryNotifiesNothing() throws {
        let controller = F.controller("if close > open\n    alert(\"Green\")")
        let bars = (0..<5_000).map { F.bar($0, open: 100, close: 101, closed: true) }
        let update = try XCTUnwrap(F.update(controller.rebuild(bars: bars, live: false)))
        var frequencyGuard = PineAlertFrequencyGuard()
        XCTAssertTrue(route(update, [subscription()], guard: &frequencyGuard).isEmpty)
    }

    func testGreenCloseNotifiesOnceAndRedNever() throws {
        let controller = F.controller(
            "if close > open\n    alert(\"Long \" + syminfo.ticker + \" @ \" + str.tostring(close), "
                + "alert.freq_once_per_bar_close)")
        _ = controller.rebuild(
            bars: F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)], live: true, now: F.now(during: 3))
        var frequencyGuard = PineAlertFrequencyGuard()
        let subscriptions = [subscription()]

        let green = try XCTUnwrap(ticks(controller, bar: 3, open: 100, closes: [101]).first)
        let routed = route(green, subscriptions, guard: &frequencyGuard)
        XCTAssertEqual(routed.count, 1)
        XCTAssertTrue(routed[0].notification.message.hasPrefix("Long "))
        XCTAssertTrue(routed[0].notification.message.contains("101"))
        XCTAssertTrue(routed[0].notification.isConfirmed)

        // The same closed bar evaluated again does not notify twice.
        XCTAssertTrue(route(green, subscriptions, guard: &frequencyGuard).isEmpty)

        let red = try XCTUnwrap(ticks(controller, bar: 4, open: 103, closes: [102]).first)
        XCTAssertTrue(route(red, subscriptions, guard: &frequencyGuard).isEmpty)
    }

    func testFrequenciesFollowTheLifecycleThroughTheRouter() throws {
        XCTAssertEqual(try deliveries("all"), [1, 1, 1, 1])
        XCTAssertEqual(try deliveries("once_per_bar"), [1, 0, 0, 1])
        XCTAssertEqual(try deliveries("once_per_bar_close"), [0, 0, 1, 0])
    }

    func testNonRealtimeEventsAreDropped() {
        let event = PineAlertEvent(
            id: 1, site: 1, bar: 3, time: F.time(3), message: "old", frequency: .all, isRealtime: false,
            isConfirmed: true)
        var frequencyGuard = PineAlertFrequencyGuard()
        let routed = PineAlertRouter.route(
            events: [event], barID: PineBarID(dataset: F.dataset, openTime: F.time(3)), chartID: chartID,
            sourceHash: sourceHash, subscriptions: [subscription()], guard: &frequencyGuard)
        XCTAssertTrue(routed.isEmpty)
    }

    func testBarThatEndedBeforeTheSubscriptionIsDropped() throws {
        let controller = F.controller("if close > open\n    alert(\"Up\", alert.freq_all)")
        _ = controller.rebuild(
            bars: F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)], live: true, now: F.now(during: 3))
        var update = try XCTUnwrap(ticks(controller, bar: 3, open: 100, closes: [101], closingLast: false).first)
        update.barID = update.barID.map {
            PineBarID(dataset: PineDatasetKey(symbolKey: $0.dataset.symbolKey, timeframe: "1H"), openTime: $0.openTime)
        }
        var frequencyGuard = PineAlertFrequencyGuard()
        let late = subscription(timeframe: "1H", createdAt: F.time(3).addingTimeInterval(2 * 3_600))
        XCTAssertTrue(route(update, [late], guard: &frequencyGuard).isEmpty)
        let early = subscription(timeframe: "1H", createdAt: F.time(3).addingTimeInterval(60))
        XCTAssertEqual(route(update, [early], guard: &frequencyGuard).count, 1)
    }

    // MARK: - Matching

    func testOnlyMatchingSubscriptionsFire() throws {
        let controller = F.controller("if close > open\n    alert(\"Up\", alert.freq_all)")
        _ = controller.rebuild(
            bars: F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)], live: true, now: F.now(during: 3))
        let update = try XCTUnwrap(ticks(controller, bar: 3, open: 100, closes: [101], closingLast: false).first)
        let match = subscription()
        let subscriptions = [
            match,
            subscription(chartID: UUID()),
            subscription(symbolKey: "binance:ETH"),
            subscription(timeframe: "1D"),
            subscription(sourceHash: "hash-b"),
            subscription(state: .paused),
            subscription(state: .scriptChanged),
            subscription(state: .scriptDeleted),
        ]
        var frequencyGuard = PineAlertFrequencyGuard()
        let routed = route(update, subscriptions, guard: &frequencyGuard)
        XCTAssertEqual(routed.map(\.notification.subscriptionID), [match.id])
    }

    func testSubscriptionsDoNotSuppressEachOther() throws {
        let controller = F.controller("if close > open\n    alert(\"Up\", alert.freq_once_per_bar)")
        _ = controller.rebuild(
            bars: F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)], live: true, now: F.now(during: 3))
        let update = try XCTUnwrap(ticks(controller, bar: 3, open: 100, closes: [101], closingLast: false).first)
        var frequencyGuard = PineAlertFrequencyGuard()
        let routed = route(update, [subscription(), subscription()], guard: &frequencyGuard)
        XCTAssertEqual(routed.count, 2)
        XCTAssertEqual(Set(routed.compactMap(\.dedupeKey)).count, 2)
    }

    func testNoSourceOrBarMeansNothingRoutes() {
        var frequencyGuard = PineAlertFrequencyGuard()
        let event = PineAlertEvent(
            id: 1, site: 1, bar: 3, time: F.time(3), message: "x", frequency: .all, isRealtime: true, isConfirmed: true)
        let subscriptions = [subscription()]
        XCTAssertTrue(
            PineAlertRouter.route(
                events: [event], barID: nil, chartID: chartID, sourceHash: sourceHash, subscriptions: subscriptions,
                guard: &frequencyGuard
            ).isEmpty)
        XCTAssertTrue(
            PineAlertRouter.route(
                events: [event], barID: PineBarID(dataset: F.dataset, openTime: F.time(3)), chartID: chartID,
                sourceHash: nil, subscriptions: subscriptions, guard: &frequencyGuard
            ).isEmpty)
    }

    // MARK: - Guard

    func testCloseFrequencyRequiresAConfirmedRealtimeEvent() {
        var frequencyGuard = PineAlertFrequencyGuard()
        let id = UUID()
        func event(confirmed: Bool, realtime: Bool = true) -> PineAlertEvent {
            PineAlertEvent(
                id: 1, site: 1, bar: 1, time: F.time(1), message: "", frequency: .oncePerBarClose,
                isRealtime: realtime, isConfirmed: confirmed)
        }
        XCTAssertFalse(frequencyGuard.admit(event(confirmed: false), subscription: id))
        XCTAssertFalse(frequencyGuard.admit(event(confirmed: true, realtime: false), subscription: id))
        XCTAssertTrue(frequencyGuard.admit(event(confirmed: true), subscription: id))
        XCTAssertFalse(frequencyGuard.admit(event(confirmed: true), subscription: id))
    }

    func testGuardKeepsCallSitesAndBarsApart() {
        var frequencyGuard = PineAlertFrequencyGuard()
        let id = UUID()
        func event(site: Int, bar: Int) -> PineAlertEvent {
            PineAlertEvent(
                id: 1, site: site, bar: bar, time: F.time(bar), message: "", frequency: .oncePerBar,
                isRealtime: true, isConfirmed: false)
        }
        XCTAssertTrue(frequencyGuard.admit(event(site: 1, bar: 1), subscription: id))
        XCTAssertTrue(frequencyGuard.admit(event(site: 2, bar: 1), subscription: id))
        XCTAssertTrue(frequencyGuard.admit(event(site: 1, bar: 2), subscription: id))
        XCTAssertFalse(frequencyGuard.admit(event(site: 1, bar: 1), subscription: id))
    }

    func testGuardForgetsASubscriptionOnReset() {
        var frequencyGuard = PineAlertFrequencyGuard()
        let id = UUID()
        let other = UUID()
        let event = PineAlertEvent(
            id: 1, site: 1, bar: 1, time: F.time(1), message: "", frequency: .oncePerBar, isRealtime: true,
            isConfirmed: false)
        XCTAssertTrue(frequencyGuard.admit(event, subscription: id))
        XCTAssertTrue(frequencyGuard.admit(event, subscription: other))
        frequencyGuard.reset(subscription: id)
        XCTAssertTrue(frequencyGuard.admit(event, subscription: id))
        XCTAssertFalse(frequencyGuard.admit(event, subscription: other))
    }

    func testGuardSeededFromTheStoreSurvivesARestart() throws {
        let database = try AppDatabase.makeInMemory()
        let id = UUID()
        let event = PineAlertEvent(
            id: 1, site: 1, bar: 3, time: F.time(3), message: "Up", frequency: .oncePerBar, isRealtime: true,
            isConfirmed: false)
        let key = try XCTUnwrap(PineAlertFrequencyGuard.dedupeKey(subscription: id, event: event))
        let delivered = PineAlertNotification(
            subscriptionID: id, scriptName: "S", chartID: chartID, symbolKey: F.dataset.symbolKey,
            timeframe: F.dataset.timeframe, barTime: event.time, message: "Up", frequency: .oncePerBar,
            isConfirmed: false)
        XCTAssertTrue(database.insertPineAlertEvent(delivered, dedupeKey: key))

        // "Relaunch": a new guard knows only what the database remembers.
        var restarted = PineAlertFrequencyGuard(seeded: database.recentPineAlertDedupeKeys())
        XCTAssertFalse(restarted.admit(event, subscription: id))
    }

    // MARK: - Coordinator

    private func waitForDeliveries(_ channel: RecordingPineAlertChannel, count: Int) async {
        for _ in 0..<100 where channel.delivered.count < count {
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    private func appliedChart(_ source: String) throws -> ChartViewModel {
        let chart = ChartViewModel(ticker: "BTC")
        chart.updatePineDraft("//@version=6\nindicator(\"T\")\n\(source)")
        XCTAssertTrue(chart.applyPineDraft())
        return chart
    }

    func testLoadPineScriptAppliesAndUnloadForgetsIt() throws {
        let chart = ChartViewModel(ticker: "BTC")
        XCTAssertTrue(chart.loadPineScript(source: "//@version=6\nindicator(\"T\")\nplot(close)\n"))
        XCTAssertNotNil(chart.appliedSourceHash)

        chart.scriptInstances = [ChartScriptInstance(scriptID: UUID(), loadedRevisionID: UUID())]
        chart.unloadPineScript()
        XCTAssertNil(chart.pineConfiguration)
        XCTAssertNil(chart.appliedSourceHash)
        XCTAssertTrue(chart.scriptInstances.isEmpty)
        XCTAssertTrue(chart.pineDiagnostics.isEmpty)
        XCTAssertEqual(chart.pineStatus, "No script applied")
    }

    func testLoadPineScriptKeepsThePreviousScriptWhenTheNewOneFails() throws {
        let chart = try appliedChart("plot(close)")
        let applied = chart.pineConfiguration?.appliedSource
        XCTAssertFalse(chart.loadPineScript(source: "plot(("))
        XCTAssertEqual(chart.pineConfiguration?.appliedSource, applied)
        XCTAssertFalse(chart.pineDiagnostics.isEmpty)
    }

    func testCoordinatorDeliversThenGoesQuietWhenTheScriptChanges() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let channel = RecordingPineAlertChannel()
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: [channel]))
        let chart = try appliedChart("if close > open\n    alert(\"Up\", alert.freq_all)")
        coordinator.attach(chart)
        let subscription = try XCTUnwrap(
            coordinator.subscribe(chart, scriptID: nil, scriptName: "Script", note: ""))
        let dataset = chart.pineAlertDataset
        let appliedHash = try XCTUnwrap(chart.appliedSourceHash)

        let event = PineAlertEvent(
            id: 1, site: 1, bar: 3, time: Date(), message: "Up", frequency: .all, isRealtime: true, isConfirmed: false)
        let barID = PineBarID(dataset: dataset, openTime: event.time)
        coordinator.ingest(events: [event], barID: barID, chartID: chart.chartID, sourceHash: appliedHash)
        await waitForDeliveries(channel, count: 1)
        XCTAssertEqual(channel.delivered.count, 1)
        XCTAssertEqual(store.history.count, 1)

        // The script is edited: paused, silent.
        coordinator.contextChanged(chartID: chart.chartID, dataset: dataset, sourceHash: "edited")
        XCTAssertEqual(store.subscription(id: subscription.id)?.state, .scriptChanged)
        coordinator.ingest(events: [event], barID: barID, chartID: chart.chartID, sourceHash: "edited")
        try? await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(channel.delivered.count, 1)

        // Re-arming pins the chart's current script again.
        XCTAssertTrue(coordinator.rearm(subscription: subscription.id))
        XCTAssertEqual(store.subscription(id: subscription.id)?.state, .active)
        XCTAssertEqual(store.subscription(id: subscription.id)?.sourceHash, appliedHash)
    }

    func testPausedSubscriptionStaysSilentAndChartRemovalDeletesIt() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let channel = RecordingPineAlertChannel()
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: [channel]))
        let chart = try appliedChart("alert(\"x\", alert.freq_all)")
        coordinator.attach(chart)
        let subscription = try XCTUnwrap(
            coordinator.subscribe(chart, scriptID: nil, scriptName: "Script", note: ""))
        coordinator.pause(subscription: subscription.id)

        let event = PineAlertEvent(
            id: 1, site: 1, bar: 3, time: Date(), message: "x", frequency: .all, isRealtime: true, isConfirmed: false)
        coordinator.ingest(
            events: [event], barID: PineBarID(dataset: chart.pineAlertDataset, openTime: event.time),
            chartID: chart.chartID, sourceHash: chart.appliedSourceHash)
        try? await Task.sleep(for: .milliseconds(100))
        XCTAssertTrue(channel.delivered.isEmpty)

        coordinator.chartRemoved(chartID: chart.chartID)
        XCTAssertTrue(store.subscriptions.isEmpty)
    }

    func testRearmNeedsAnOpenChart() throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: []))
        let orphan = subscription(state: .scriptChanged)
        store.add(orphan)
        XCTAssertFalse(coordinator.canRearm(orphan))
        XCTAssertFalse(coordinator.rearm(subscription: orphan.id))
        XCTAssertEqual(store.subscription(id: orphan.id)?.state, .scriptChanged)
    }
}
