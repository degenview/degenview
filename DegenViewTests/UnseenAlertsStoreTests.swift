import XCTest

@testable import DegenView

@MainActor
final class UnseenAlertsStoreTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_800_000_000)
    private var clock = Date(timeIntervalSince1970: 1_800_000_000)
    private var bus = AlertEventBus()
    private var database: AppDatabase!

    override func setUpWithError() throws {
        clock = start
        bus = AlertEventBus()
        database = try AppDatabase.makeInMemory()
    }

    private func makeStore() -> UnseenAlertsStore {
        UnseenAlertsStore(bus: bus, database: database, now: { [unowned self] in clock })
    }

    private func price(_ id: UUID = UUID(), after seconds: TimeInterval = 10) -> AlertDomainEvent {
        .priceAlertTriggered(eventID: id, at: start.addingTimeInterval(seconds))
    }

    private func script(_ id: UUID = UUID(), after seconds: TimeInterval = 10) -> AlertDomainEvent {
        .scriptAlertTriggered(eventID: id, at: start.addingTimeInterval(seconds))
    }

    func testPriceAndScriptAlertsBothCount() {
        let store = makeStore()
        bus.publish(price())
        bus.publish(script())
        XCTAssertEqual(store.count, 2)
    }

    func testRedeliveredEventCountsOnce() {
        let store = makeStore()
        let id = UUID()
        bus.publish(price(id))
        bus.publish(price(id))
        XCTAssertEqual(store.count, 1)
    }

    func testEventsFromBeforeFirstRunAreIgnored() {
        let store = makeStore()
        bus.publish(price(after: -60))
        XCTAssertEqual(store.count, 0)
    }

    func testOpeningTheWindowClearsTheCount() {
        let store = makeStore()
        bus.publish(price())
        clock = start.addingTimeInterval(20)
        store.windowDidBecomeActive()
        XCTAssertEqual(store.count, 0)
    }

    func testSeenEventsStayGoneAfterRelaunch() {
        let store = makeStore()
        let id = UUID()
        bus.publish(price(id))
        clock = start.addingTimeInterval(20)
        store.windowDidBecomeActive()
        store.windowDidResign()

        let relaunched = makeStore()
        bus.publish(price(id))
        XCTAssertEqual(relaunched.count, 0)
    }

    func testUnseenEventsFromBeforeRelaunchCountAgain() {
        _ = makeStore()
        clock = start.addingTimeInterval(100)
        let relaunched = makeStore()
        bus.publish(price(after: 50))
        XCTAssertEqual(relaunched.count, 1)
    }

    func testEventsWhileTheWindowIsKeyAreSeenImmediately() {
        let store = makeStore()
        store.windowDidBecomeActive()
        bus.publish(price())
        XCTAssertEqual(store.count, 0)
    }

    func testCountingResumesAfterTheWindowResigns() {
        let store = makeStore()
        store.windowDidBecomeActive()
        bus.publish(price(after: 10))
        store.windowDidResign()
        bus.publish(price(after: 30))
        XCTAssertEqual(store.count, 1)
    }

    func testPineStorePublishesOnceAndNotForADuplicate() throws {
        let store = PineAlertStore(database: try AppDatabase.makeInMemory(), eventBus: bus)
        var received: [AlertDomainEvent] = []
        let subscription = bus.events.sink { received.append($0) }
        defer { subscription.cancel() }

        let notification = PineAlertNotification(
            subscriptionID: UUID(), scriptName: "S", chartID: UUID(), symbolKey: "binance:BTCUSDT",
            timeframe: "1h", barTime: start, message: "m", frequency: .oncePerBar, isConfirmed: true)
        XCTAssertTrue(store.record(notification, dedupeKey: "k"))
        var duplicate = notification
        duplicate.id = UUID()
        XCTAssertFalse(store.record(duplicate, dedupeKey: "k"))

        XCTAssertEqual(received, [.scriptAlertTriggered(eventID: notification.id, at: notification.triggeredAt)])
    }
}
