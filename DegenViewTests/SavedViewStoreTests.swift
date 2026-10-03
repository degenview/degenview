import XCTest

@testable import DegenView

@MainActor
final class SavedViewStoreTests: XCTestCase {
    private struct WriteFailure: Error {}

    private final class Clock {
        var time = Date(timeIntervalSince1970: 1_000)
        func tick() -> Date {
            time += 60
            return time
        }
    }

    private let clock = Clock()

    private func makeView(_ name: String) -> SavedView {
        SavedView(
            name: name, tickers: ["BTCUSDT"], timeRange: .oneDay, createdAt: Date(),
            tickerConfigs: [TickerConfig(symbol: "BTCUSDT", source: .binance)],
            candleCount: TimeRange.oneDay.dataPointLimit)
    }

    private func makeStore(
        database: AppDatabase? = nil, persist: (([SavedView]) throws -> Void)? = nil
    ) throws -> SavedViewStore {
        SavedViewStore(database: try database ?? .makeInMemory(), persist: persist, now: { [clock] in clock.tick() })
    }

    func testViewsWithoutNewFieldsStillDecode() throws {
        let encoded = try JSONEncoder().encode(makeView("Old"))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        object.removeValue(forKey: "lastOpenedAt")
        object.removeValue(forKey: "autosave")
        let legacy = try JSONSerialization.data(withJSONObject: object)

        let decoded = try JSONDecoder().decode(SavedView.self, from: legacy)
        XCTAssertNil(decoded.lastOpenedAt)
        XCTAssertNil(decoded.autosave)
    }

    func testUpsertPersistsAndKeepsPosition() throws {
        let database = try AppDatabase.makeInMemory()
        let store = try makeStore(database: database)
        let a = makeView("A")
        let b = makeView("B")
        try store.upsert(a)
        try store.upsert(b)

        var edited = a
        edited.name = "A2"
        try store.upsert(edited)

        XCTAssertEqual(store.views.map(\.name), ["A2", "B"])
        XCTAssertEqual(database.savedViews().map(\.name), ["A2", "B"])
    }

    func testRenameKeepsIdentityPositionAndRecency() throws {
        let store = try makeStore()
        let a = makeView("A")
        try store.upsert(a)
        try store.upsert(makeView("B"))
        let openedAt = try XCTUnwrap(store.view(id: a.id)?.lastOpenedAt)

        try store.rename(id: a.id, to: "Renamed")

        XCTAssertEqual(store.views.map(\.id).first, a.id)
        XCTAssertEqual(store.views.first?.name, "Renamed")
        XCTAssertEqual(store.view(id: a.id)?.lastOpenedAt, openedAt)
    }

    func testRecentIsNewestFirstAndBounded() throws {
        let store = try makeStore()
        let views = ["A", "B", "C"].map(makeView)
        views.forEach { try? store.upsert($0) }
        try store.markOpened(id: views[0].id)  // A is now the newest
        try store.markOpened(id: views[1].id)
        try store.markOpened(id: views[2].id)

        XCTAssertEqual(store.recent().map(\.name), ["C", "B", "A"])
        XCTAssertEqual(store.recent(limit: 2).map(\.name), ["C", "B"])
        XCTAssertEqual(store.recent(excluding: views[2].id).map(\.name), ["B", "A"])
    }

    func testViewsNeverOpenedAreNotRecent() throws {
        var legacy = makeView("Legacy")
        legacy.lastOpenedAt = nil
        let database = try AppDatabase.makeInMemory()
        database.saveSavedViews([legacy])
        let store = try makeStore(database: database)
        XCTAssertTrue(store.recent().isEmpty)
    }

    func testDeleteRemovesFromRecent() throws {
        let store = try makeStore()
        let a = makeView("A")
        try store.upsert(a)
        try store.delete(id: a.id)
        XCTAssertTrue(store.recent().isEmpty)
        XCTAssertTrue(store.views.isEmpty)
    }

    func testFailedWriteLeavesStateUntouched() throws {
        var fail = false
        let store = try makeStore { _ in if fail { throw WriteFailure() } }
        let a = makeView("A")
        try store.upsert(a)

        fail = true
        XCTAssertThrowsError(try store.rename(id: a.id, to: "Nope"))
        XCTAssertThrowsError(try store.upsert(makeView("B")))
        XCTAssertThrowsError(try store.delete(id: a.id))
        XCTAssertEqual(store.views.map(\.name), ["A"])
    }

    func testAutosaveFlagRoundTrips() throws {
        let database = try AppDatabase.makeInMemory()
        let store = try makeStore(database: database)
        let a = makeView("A")
        try store.upsert(a)
        try store.setAutosave(id: a.id, true)
        XCTAssertEqual(database.savedViews().first?.autosave, true)
        try store.setAutosave(id: a.id, false)
        XCTAssertNil(database.savedViews().first?.autosave)
    }
}
