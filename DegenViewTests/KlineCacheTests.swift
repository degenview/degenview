import XCTest
@testable import DegenView

final class KlineCacheTests: XCTestCase {
    final class ClockBox: @unchecked Sendable {
        var value: Date
        init(_ value: Date) { self.value = value }
    }

    private func candles(_ count: Int) -> [KlineData] {
        (0..<count).map { KlineData(time: Date(timeIntervalSince1970: Double($0)), price: Double($0)) }
    }

    func testFreshReadsSliceTailAndTTLBoundaryExpires() async {
        let clock = ClockBox(Date(timeIntervalSince1970: 1_000))
        let cache = KlineCache(now: { clock.value })
        await cache.set(symbol: "btc", interval: "1m", days: 1, data: candles(5))

        let fresh = await cache.get(symbol: "BTC", interval: "1m", days: 1, count: 2, ttl: 60)
        XCTAssertEqual(fresh?.map(\.closePrice), [3, 4])

        clock.value = clock.value.addingTimeInterval(60)
        let expired = await cache.get(symbol: "BTC", interval: "1m", days: 1, count: 2, ttl: 60)
        XCTAssertNil(expired)
        let stale = await cache.getStale(symbol: "BTC", interval: "1m", days: 1, count: 2)
        XCTAssertEqual(stale?.count, 2)
    }

    func testBestStaleEntryUsesSmallestCoveringWindow() async {
        let cache = KlineCache()
        await cache.set(symbol: "BTC", interval: "1m", days: 1, data: candles(10))
        await cache.set(symbol: "BTC", interval: "1h", days: 30, data: candles(20))
        let result = await cache.getAnyStale(symbol: "btc", count: 8)
        XCTAssertEqual(result?.count, 8)
        XCTAssertEqual(result?.first?.closePrice, 2)
    }
}
