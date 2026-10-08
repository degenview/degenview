import XCTest

@testable import DegenView

final class GranularReplayLoadingTests: XCTestCase {
    private let base = Date(timeIntervalSince1970: 1_700_000_000)
    private let key = GranularReplayCache.Key(symbol: "BTCUSDT", interval: .automatic)

    private func bars(_ range: Range<Int>) -> [KlineData] {
        range.map { KlineData(time: base.addingTimeInterval(Double($0) * 60), price: Double($0 + 1)) }
    }

    func testCacheServesAnyRangeInsideWhatWasFetched() {
        var cache = GranularReplayCache()
        cache.store(key, start: base, end: base.addingTimeInterval(600), data: bars(0..<10))
        let slice = cache.slice(
            key, start: base.addingTimeInterval(240), end: base.addingTimeInterval(600))
        XCTAssertEqual(slice?.count, 6)
        XCTAssertEqual(slice?.first?.openTime, base.addingTimeInterval(240))
    }

    func testCacheMissesEarlierStartLaterEndOtherKeyAndExpiry() {
        var cache = GranularReplayCache()
        let start = base.addingTimeInterval(120)
        let end = base.addingTimeInterval(600)
        cache.store(key, start: start, end: end, data: bars(2..<10), now: base)
        XCTAssertNil(cache.slice(key, start: base, end: end, now: base))
        XCTAssertNil(cache.slice(key, start: start, end: end.addingTimeInterval(60), now: base))
        XCTAssertNil(
            cache.slice(
                GranularReplayCache.Key(symbol: "ETHUSDT", interval: .automatic), start: start, end: end, now: base))
        XCTAssertNil(cache.slice(key, start: start, end: end, now: base.addingTimeInterval(301)))
        XCTAssertNotNil(cache.slice(key, start: start, end: end, now: base.addingTimeInterval(10)))
    }

    func testCacheEvictsOldestBeyondCapacity() {
        var cache = GranularReplayCache()
        for n in 0..<4 {
            let k = GranularReplayCache.Key(symbol: "S\(n)", interval: .automatic)
            cache.store(
                k, start: base, end: base.addingTimeInterval(60), data: bars(0..<1),
                now: base.addingTimeInterval(Double(n)))
        }
        let first = GranularReplayCache.Key(symbol: "S0", interval: .automatic)
        XCTAssertNil(cache.slice(first, start: base, end: base.addingTimeInterval(60), now: base.addingTimeInterval(5)))
    }

    func testPagesComeBackInOrderAndRunTogether() async throws {
        let results = try await fetchPagesConcurrently(count: 12, maxConcurrent: 4) { index in
            try await Task.sleep(nanoseconds: UInt64((12 - index) * 5_000_000))
            return index
        }
        XCTAssertEqual(results, Array(0..<12))
    }

    func testPagesRethrowTheFirstError() async {
        struct Boom: Error {}
        do {
            _ = try await fetchPagesConcurrently(count: 6) { index in
                if index == 3 { throw Boom() }
                return index
            }
            XCTFail("expected throw")
        } catch is Boom {
        } catch {
            XCTFail("wrong error \(error)")
        }
    }
}
