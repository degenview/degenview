import XCTest

@testable import DegenView

final class LocalPriceAlertEngineTests: XCTestCase {
    actor MemoryRepository: AlertSnapshotRepository {
        var value: AlertPersistenceSnapshot?
        init(_ value: AlertPersistenceSnapshot? = nil) { self.value = value }
        func load() -> AlertPersistenceSnapshot? { value }
        func save(_ snapshot: AlertPersistenceSnapshot) { value = snapshot }
    }

    private let asset = PortfolioAsset(key: "Binance:BTCUSDT", symbol: "BTC", name: "Bitcoin", source: .binance)
    private func quote(_ price: Decimal, _ sequence: Int, age: TimeInterval = 0) -> MarketQuote {
        let now = Date()
        return MarketQuote(
            asset: asset, price: price, currency: .USD, sourceTimestamp: now.addingTimeInterval(-age), receivedAt: now,
            maximumAge: 60, fingerprint: "q\(sequence)")
    }

    func testCrossAboveRequiresStrictOppositeBaselineAndIncludesEquality() async {
        let engine = await LocalPriceAlertEngine(repository: MemoryRepository())
        await engine.saveAlert(PriceAlert(asset: asset, condition: .crossesAbove(target: 100)), baseline: 90)
        let first = await engine.process(quote(99, 1))
        XCTAssertTrue(first.isEmpty)
        let second = await engine.process(quote(100, 2))
        XCTAssertEqual(second.count, 1)
        let third = await engine.process(quote(110, 3))
        XCTAssertTrue(third.isEmpty)
        let snapshot = await engine.currentSnapshot()
        XCTAssertEqual(snapshot.alerts.first?.state, .triggered)
    }

    func testRepeatingDisarmsAndRearmsOnlyOnStrictOppositeSide() async {
        let engine = await LocalPriceAlertEngine(repository: MemoryRepository())
        await engine.saveAlert(
            PriceAlert(asset: asset, condition: .crossesBelow(target: 100), frequency: .everyTime), baseline: 110)
        let first = await engine.process(quote(100, 1))
        XCTAssertEqual(first.count, 1)
        let second = await engine.process(quote(90, 2))
        XCTAssertTrue(second.isEmpty)
        let third = await engine.process(quote(100, 3))
        XCTAssertTrue(third.isEmpty)
        let fourth = await engine.process(quote(101, 4))
        XCTAssertTrue(fourth.isEmpty)
        let fifth = await engine.process(quote(99, 5))
        XCTAssertEqual(fifth.count, 1)
    }

    func testCreationOnTriggeredSideWaitsForRearm() async {
        let engine = await LocalPriceAlertEngine(repository: MemoryRepository())
        await engine.saveAlert(
            PriceAlert(asset: asset, condition: .crossesAbove(target: 100), frequency: .everyTime), baseline: 110)
        let first = await engine.process(quote(120, 1))
        XCTAssertTrue(first.isEmpty)
        let second = await engine.process(quote(99, 2))
        XCTAssertTrue(second.isEmpty)
        let third = await engine.process(quote(100, 3))
        XCTAssertEqual(third.count, 1)
    }

    func testStaleAndDuplicateQuotesDoNotTrigger() async {
        let engine = await LocalPriceAlertEngine(repository: MemoryRepository())
        await engine.saveAlert(PriceAlert(asset: asset, condition: .crossesAbove(target: 100)), baseline: 90)
        let stale = await engine.process(quote(110, 1, age: 120))
        XCTAssertTrue(stale.isEmpty)
        let fresh = await engine.process(quote(110, 2))
        XCTAssertEqual(fresh.count, 1)
        let duplicate = await engine.process(quote(110, 2))
        XCTAssertTrue(duplicate.isEmpty)
    }

    func testHistoryAndRulesHaveIndependentDeletion() async {
        let engine = await LocalPriceAlertEngine(repository: MemoryRepository())
        let alert = PriceAlert(asset: asset, condition: .crossesAbove(target: 100))
        await engine.saveAlert(alert, baseline: 90)
        _ = await engine.process(quote(100, 1))
        await engine.delete(alert.id)
        let retained = await engine.currentSnapshot()
        XCTAssertEqual(retained.history.count, 1)
        await engine.clearHistory()
        let cleared = await engine.currentSnapshot()
        XCTAssertTrue(cleared.history.isEmpty)
    }

    func testMultipleActiveAlertsForSameAssetProduceOneSubscriptionAsset() async {
        let engine = await LocalPriceAlertEngine(repository: MemoryRepository())
        await engine.saveAlert(PriceAlert(asset: asset, condition: .crossesAbove(target: 100)), baseline: 90)
        await engine.saveAlert(PriceAlert(asset: asset, condition: .crossesBelow(target: 80)), baseline: 90)

        let assets = await engine.activeAssets()

        XCTAssertEqual(assets.count, 1)
        XCTAssertEqual(assets.first?.key, asset.key)
    }

    func testQuoteCoordinatorDeduplicatesAssetsAcrossOwners() {
        let duplicate = PortfolioAsset(key: asset.key, symbol: "XBT", name: "Duplicate", source: .binance)
        let values = MarketQuoteCoordinator.assetsByKey([asset, duplicate])

        XCTAssertEqual(values.count, 1)
        XCTAssertEqual(values[asset.key]?.symbol, asset.symbol)
    }

    func testDuplicateCommandIsAppliedOnce() async {
        let engine = await LocalPriceAlertEngine(repository: MemoryRepository())
        let alert = PriceAlert(asset: asset, condition: .crossesAbove(target: 100))
        let command = AlertRuntimeCommand(payload: .save(alert, baseline: 90))
        await engine.apply(command)
        await engine.apply(command)
        let snapshot = await engine.currentSnapshot()
        XCTAssertEqual(snapshot.alerts.count, 1)
        XCTAssertEqual(snapshot.processedCommandIDs, [command.id])
    }

    func testCatchUpIsChronologicalAndLimitedToOneEventPerAlert() async {
        let engine = await LocalPriceAlertEngine(repository: MemoryRepository())
        await engine.saveAlert(
            PriceAlert(asset: asset, condition: .crossesAbove(target: 100), frequency: .everyTime), baseline: 90)
        let now = Date()
        func historical(_ price: Decimal, minutes: Double, fingerprint: String) -> MarketQuote {
            let date = now.addingTimeInterval(minutes * 60)
            return MarketQuote(
                asset: asset, price: price, currency: .USD, sourceTimestamp: date, receivedAt: date, maximumAge: 86_400,
                fingerprint: fingerprint)
        }
        let events = await engine.processCatchUp(
            [
                historical(110, minutes: -10, fingerprint: "later"),
                historical(90, minutes: -20, fingerprint: "earlier"),
                historical(90, minutes: -5, fingerprint: "rearm"),
                historical(110, minutes: -1, fingerprint: "second-crossing"),
            ], now: now)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events.first?.origin, .catchUp)
        let snapshot = await engine.currentSnapshot()
        XCTAssertEqual(snapshot.history.count, 1)
    }

    func testSchemaOneSnapshotDecodesWithMigrationDefaults() throws {
        let old = AlertPersistenceSnapshot()
        let encoded = try JSONEncoder().encode(old)
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        object["schemaVersion"] = 1
        ["revision", "processedCommandIDs", "health"].forEach { object.removeValue(forKey: $0) }
        let migrated = try JSONDecoder().decode(
            AlertPersistenceSnapshot.self, from: JSONSerialization.data(withJSONObject: object))
        XCTAssertEqual(migrated.schemaVersion, AlertPersistenceSnapshot.currentSchemaVersion)
        XCTAssertEqual(migrated.revision, 0)
        XCTAssertTrue(migrated.processedCommandIDs.isEmpty)
    }
}
