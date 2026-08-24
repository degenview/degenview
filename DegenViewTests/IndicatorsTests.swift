import XCTest
@testable import DegenView

final class IndicatorsTests: XCTestCase {
    private func candles(_ prices: [Double]) -> [KlineData] {
        prices.enumerated().map { KlineData(time: Date(timeIntervalSince1970: Double($0.offset)), price: $0.element) }
    }

    func testEMAUsesSeedAverageAndMaintainsAlignment() {
        let values = candles([1, 2, 3, 4, 5]).ema(period: 3)
        XCTAssertEqual(values.count, 5)
        XCTAssertNil(values[0])
        XCTAssertNil(values[1])
        XCTAssertEqual(values[2], 2)
        XCTAssertEqual(values[3], 3)
        XCTAssertEqual(values[4], 4)
    }

    func testRSIHandlesMonotonicAndFlatSeries() {
        XCTAssertEqual(candles(Array(1...20).map(Double.init)).rsi().last!, 100)
        XCTAssertEqual(candles(Array((1...20).reversed()).map(Double.init)).rsi().last!, 0)
        XCTAssertEqual(candles(Array(repeating: 5, count: 20)).rsi().last!, 50)
    }

    func testBollingerWarmupAndKnownWindow() {
        let bands = candles([1, 2, 3, 4, 5]).bollingerBands(period: 5, multiplier: 2)
        XCTAssertEqual(bands.middle.count, 5)
        XCTAssertEqual(bands.middle.last!, 3)
        XCTAssertEqual(bands.upper[4]!, 3 + 2 * sqrt(2), accuracy: 0.0001)
        XCTAssertEqual(bands.lower[4]!, 3 - 2 * sqrt(2), accuracy: 0.0001)
    }
}
