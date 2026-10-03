import XCTest

@testable import DegenView

@MainActor
final class PineAlertRoutingTests: XCTestCase {
    private typealias F = PineExecutionFixtures

    private let chartID = UUID()
    private let sourceHash = "hash-a"

    private func subscription(
        chartID: UUID? = nil, instanceID: UUID? = nil, symbolKey: String = F.dataset.symbolKey,
        timeframe: String = F.dataset.timeframe, sourceHash: String? = nil,
        state: PineAlertSubscription.State = .active, createdAt: Date = Date(timeIntervalSince1970: 0)
    ) -> PineAlertSubscription {
        PineAlertSubscription(
            chartID: chartID ?? self.chartID, instanceID: instanceID, scriptName: "Script", symbolKey: symbolKey,
            timeframe: timeframe,
            sourceHash: sourceHash ?? self.sourceHash, state: state, createdAt: createdAt)
    }

    private func route(
        _ update: PineExecutionUpdate, _ subscriptions: [PineAlertSubscription],
        guard frequencyGuard: inout PineAlertFrequencyGuard, instanceID: UUID? = nil, sourceHash: String? = "hash-a"
    ) -> [PineAlertRouter.Routed] {
        PineAlertRouter.route(
            events: update.alerts, barID: update.barID, chartID: chartID, instanceID: instanceID,
            sourceHash: sourceHash,
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
            instanceID: nil, sourceHash: sourceHash, subscriptions: [subscription()], guard: &frequencyGuard)
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
                events: [event], barID: nil, chartID: chartID, instanceID: nil, sourceHash: sourceHash,
                subscriptions: subscriptions, guard: &frequencyGuard
            ).isEmpty)
        XCTAssertTrue(
            PineAlertRouter.route(
                events: [event], barID: PineBarID(dataset: F.dataset, openTime: F.time(3)), chartID: chartID,
                instanceID: nil, sourceHash: nil, subscriptions: subscriptions, guard: &frequencyGuard
            ).isEmpty)
    }

    func testOnlyTheRaisingIndicatorsSubscriptionFires() throws {
        let controller = F.controller("if close > open\n    alert(\"Up\", alert.freq_all)")
        _ = controller.rebuild(
            bars: F.history([100, 100, 100]) + [F.bar(3, open: 100, close: 99)], live: true, now: F.now(during: 3))
        let update = try XCTUnwrap(
            F.update(controller.ingest(F.stream(F.bar(3, open: 100, close: 102, closed: false)))))
        let first = UUID()
        let second = UUID()
        let subscriptions = [subscription(instanceID: first), subscription(instanceID: second)]
        var frequencyGuard = PineAlertFrequencyGuard()
        let routed = route(update, subscriptions, guard: &frequencyGuard, instanceID: second)
        XCTAssertEqual(routed.map(\.notification.subscriptionID), [subscriptions[1].id])
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

    private func waitUntil(_ condition: @autoclosure () -> Bool, timeout: Duration = .seconds(3)) async {
        let deadline = ContinuousClock.now + timeout
        while !condition(), ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
    }

    private func appliedChart(_ source: String) throws -> (chart: ChartViewModel, instanceID: UUID) {
        let chart = ChartViewModel(ticker: "BTC")
        let id = try XCTUnwrap(
            chart.addPineInstance(
                scriptID: UUID(), revisionID: UUID(), source: "//@version=6\nindicator(\"T\")\n\(source)"))
        return (chart, id)
    }

    /// An applied chart whose indicator has resolved its source, so it has a hash to subscribe against.
    private func resolvedChart(_ source: String) async throws -> (chart: ChartViewModel, instanceID: UUID) {
        let applied = try appliedChart(source)
        await waitUntil(applied.chart.pineInstanceSourceHashes[applied.instanceID] != nil)
        return applied
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
        let chart = ChartViewModel(ticker: "BTC")
        XCTAssertTrue(chart.loadPineScript(source: "//@version=6\nindicator(\"T\")\nplot(close)\n"))
        let applied = chart.pineConfiguration?.appliedSource
        XCTAssertFalse(chart.loadPineScript(source: "plot(("))
        XCTAssertEqual(chart.pineConfiguration?.appliedSource, applied)
        XCTAssertFalse(chart.pineDiagnostics.isEmpty)
    }

    func testCoordinatorDeliversThenGoesQuietWhenTheScriptChanges() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let channel = RecordingPineAlertChannel()
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: [channel]))
        let (chart, instanceID) = try await resolvedChart("if close > open\n    alert(\"Up\", alert.freq_all)")
        coordinator.attach(chart)
        let subscription = try XCTUnwrap(
            coordinator.subscribe(chart, instanceID: instanceID, scriptName: "Script", note: ""))
        let dataset = chart.pineAlertDataset
        let appliedHash = try XCTUnwrap(chart.pineInstanceSourceHashes[instanceID])

        let event = PineAlertEvent(
            id: 1, site: 1, bar: 3, time: Date(), message: "Up", frequency: .all, isRealtime: true, isConfirmed: false)
        let barID = PineBarID(dataset: dataset, openTime: event.time)
        coordinator.ingest(
            events: [event], barID: barID, chartID: chart.chartID, instanceID: instanceID, sourceHash: appliedHash)
        await waitForDeliveries(channel, count: 1)
        XCTAssertEqual(channel.delivered.count, 1)
        XCTAssertEqual(store.history.count, 1)

        // The script is edited: paused, silent.
        coordinator.contextChanged(chartID: chart.chartID, dataset: dataset, hashes: [instanceID: "edited"])
        XCTAssertEqual(store.subscription(id: subscription.id)?.state, .scriptChanged)
        coordinator.ingest(
            events: [event], barID: barID, chartID: chart.chartID, instanceID: instanceID, sourceHash: "edited")
        try? await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(channel.delivered.count, 1)

        // Re-arming pins the indicator's current script again.
        XCTAssertTrue(coordinator.rearm(subscription: subscription.id))
        XCTAssertEqual(store.subscription(id: subscription.id)?.state, .active)
        XCTAssertEqual(store.subscription(id: subscription.id)?.sourceHash, appliedHash)
    }

    func testAnIndicatorStillLoadingNeverPausesItsAlert() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: []))
        let (chart, instanceID) = try await resolvedChart("alert(\"x\", alert.freq_all)")
        let subscription = try XCTUnwrap(
            coordinator.subscribe(chart, instanceID: instanceID, scriptName: "Script", note: ""))

        // No hash for it (source still resolving) and an unrelated indicator's hash: nothing changes.
        coordinator.contextChanged(chartID: chart.chartID, dataset: chart.pineAlertDataset, hashes: [:])
        coordinator.contextChanged(chartID: chart.chartID, dataset: chart.pineAlertDataset, hashes: [UUID(): "other"])
        XCTAssertEqual(store.subscription(id: subscription.id)?.state, .active)
    }

    func testTwoIndicatorsOfOneScriptHaveSeparateAlerts() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: []))
        let chart = ChartViewModel(ticker: "BTC")
        let scriptID = UUID()
        let source = "//@version=6\nindicator(\"T\")\nalert(\"x\", alert.freq_all)"
        let first = try XCTUnwrap(chart.addPineInstance(scriptID: scriptID, revisionID: UUID(), source: source))
        let second = try XCTUnwrap(chart.addPineInstance(scriptID: scriptID, revisionID: UUID(), source: source))
        await waitUntil(chart.pineInstanceSourceHashes.count == 2)

        let armed = try XCTUnwrap(coordinator.subscribe(chart, instanceID: first, scriptName: "T", note: ""))
        XCTAssertEqual(armed.instanceID, first)
        XCTAssertEqual(armed.scriptID, scriptID)
        let dataset = chart.pineAlertDataset
        XCTAssertEqual(store.subscription(forChart: chart.chartID, instanceID: first, dataset: dataset)?.id, armed.id)
        XCTAssertNil(store.subscription(forChart: chart.chartID, instanceID: second, dataset: dataset))
    }

    func testRemovingAnIndicatorDeletesItsAlertAndKeepsTheOthers() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: []))
        let chart = ChartViewModel(ticker: "BTC")
        let source = "//@version=6\nindicator(\"T\")\nalert(\"x\", alert.freq_all)"
        let first = try XCTUnwrap(chart.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: source))
        let second = try XCTUnwrap(chart.addPineInstance(scriptID: UUID(), revisionID: UUID(), source: source))
        await waitUntil(chart.pineInstanceSourceHashes.count == 2)
        coordinator.attach(chart)
        let removed = try XCTUnwrap(coordinator.subscribe(chart, instanceID: first, scriptName: "T", note: ""))
        let kept = try XCTUnwrap(coordinator.subscribe(chart, instanceID: second, scriptName: "T", note: ""))

        chart.removePineInstance(first)

        XCTAssertNil(store.subscription(id: removed.id))
        XCTAssertEqual(store.subscription(id: kept.id)?.state, .active)
    }

    func testPausedSubscriptionStaysSilentAndChartRemovalDeletesIt() async throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let channel = RecordingPineAlertChannel()
        let coordinator = PineAlertCoordinator(store: store, dispatcher: PineAlertDispatcher(channels: [channel]))
        let (chart, instanceID) = try await resolvedChart("alert(\"x\", alert.freq_all)")
        coordinator.attach(chart)
        let subscription = try XCTUnwrap(
            coordinator.subscribe(chart, instanceID: instanceID, scriptName: "Script", note: ""))
        coordinator.pause(subscription: subscription.id)

        let event = PineAlertEvent(
            id: 1, site: 1, bar: 3, time: Date(), message: "x", frequency: .all, isRealtime: true, isConfirmed: false)
        coordinator.ingest(
            events: [event], barID: PineBarID(dataset: chart.pineAlertDataset, openTime: event.time),
            chartID: chart.chartID, instanceID: instanceID, sourceHash: chart.pineInstanceSourceHashes[instanceID])
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

    func testStoreFindsTheChartsAlertForAnIndicatorOnItsCurrentMarket() throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory())
        let instance = UUID()
        func armed(
            instance: UUID? = instance, chart: UUID? = nil, timeframe: String = F.dataset.timeframe,
            state: PineAlertSubscription.State = .active, createdAt: Date = Date(timeIntervalSince1970: 0)
        ) -> PineAlertSubscription {
            subscription(
                chartID: chart, instanceID: instance, timeframe: timeframe, state: state, createdAt: createdAt)
        }
        func found() -> PineAlertSubscription? {
            store.subscription(forChart: chartID, instanceID: instance, dataset: F.dataset)
        }
        XCTAssertNil(found())

        // Another indicator, another chart and another timeframe are not this alert.
        store.add(armed(instance: UUID()))
        store.add(armed(chart: UUID()))
        store.add(armed(timeframe: "never"))
        XCTAssertNil(found())

        // A paused or script-changed alert still counts: it is re-armed, not replaced.
        let paused = armed(state: .paused)
        store.add(paused)
        XCTAssertEqual(found()?.id, paused.id)
        let changed = armed(state: .scriptChanged, createdAt: Date(timeIntervalSince1970: 10))
        store.add(changed)
        XCTAssertEqual(found()?.id, changed.id, "the newest wins when none is active")

        // An active one wins over older and newer inactive ones.
        let active = armed(createdAt: Date(timeIntervalSince1970: 5))
        store.add(active)
        XCTAssertEqual(found()?.id, active.id)
    }
}
