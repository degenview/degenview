import XCTest

@testable import DegenView

final class FibonacciRetracementTests: XCTestCase {
    func testDocumentedCanonicalLinearOrientationLowToHigh() throws {
        let fixtures: [(String, Double)] = [
            ("0", 100), ("0.236", 123.6), ("0.382", 138.2), ("0.5", 150),
            ("0.618", 161.8), ("0.786", 178.6), ("1", 200),
            ("1.618", 261.8), ("-0.236", 76.4),
        ]
        for (ratio, expected) in fixtures {
            let actual = try FibonacciCalculator.price(
                point1: 100, point2: 200, ratio: Decimal(string: ratio)!, reverse: false, mode: .linear)
            XCTAssertEqual(actual, expected, accuracy: 0.000_000_1, "ratio \(ratio)")
        }
    }

    func testDocumentedCanonicalLinearOrientationHighToLow() throws {
        XCTAssertEqual(
            try FibonacciCalculator.price(point1: 200, point2: 100, ratio: 0, reverse: false, mode: .linear),
            200)
        XCTAssertEqual(
            try FibonacciCalculator.price(point1: 200, point2: 100, ratio: 1, reverse: false, mode: .linear),
            100)
        XCTAssertEqual(
            try FibonacciCalculator.price(
                point1: 200, point2: 100, ratio: Decimal(string: "1.618")!, reverse: false, mode: .linear),
            38.2, accuracy: 0.000_000_1)
    }

    func testReverseReflectsMappingWithoutAnchorMutation() throws {
        let normal = try FibonacciCalculator.price(
            point1: 100, point2: 200, ratio: Decimal(string: "0.236")!, reverse: false, mode: .linear)
        let reversed = try FibonacciCalculator.price(
            point1: 100, point2: 200, ratio: Decimal(string: "0.236")!, reverse: true, mode: .linear)
        let restored = try FibonacciCalculator.price(
            point1: 100, point2: 200, ratio: Decimal(string: "0.236")!, reverse: false, mode: .linear)
        XCTAssertEqual(normal, 123.6, accuracy: 0.000_000_1)
        XCTAssertEqual(reversed, 176.4, accuracy: 0.000_000_1)
        XCTAssertEqual(restored, normal)
    }

    func testLogarithmicInterpolationAndExtension() throws {
        XCTAssertEqual(
            try FibonacciCalculator.price(point1: 100, point2: 1_000, ratio: 0, reverse: false, mode: .logarithmic),
            100, accuracy: 0.000_000_1)
        XCTAssertEqual(
            try FibonacciCalculator.price(
                point1: 100, point2: 1_000, ratio: Decimal(string: "0.5")!, reverse: false,
                mode: .logarithmic),
            sqrt(100_000), accuracy: 0.000_000_1)
        XCTAssertEqual(
            try FibonacciCalculator.price(point1: 100, point2: 1_000, ratio: 1, reverse: false, mode: .logarithmic),
            1_000, accuracy: 0.000_001)
        XCTAssertNotEqual(
            try FibonacciCalculator.price(
                point1: 100, point2: 1_000, ratio: Decimal(string: "0.5")!, reverse: false,
                mode: .logarithmic),
            550)
    }

    func testInvalidLogAnchorsNeverProduceGeometry() {
        XCTAssertThrowsError(
            try FibonacciCalculator.price(point1: 0, point2: 100, ratio: 0.5, reverse: false, mode: .logarithmic))
        XCTAssertThrowsError(
            try FibonacciCalculator.price(point1: -50, point2: 100, ratio: 0.5, reverse: false, mode: .logarithmic))
    }

    func testLevelLimitAndPersistenceRoundTrip() throws {
        var drawing = FibonacciRetracementDrawing(
            point1: TrendAnchor(date: .distantPast, price: 100),
            point2: TrendAnchor(date: .distantFuture, price: 200), levels: [])
        for index in 0..<FibonacciDefaults.maximumLevelCount {
            XCTAssertTrue(drawing.addLevel(FibonacciLevel(ratio: Decimal(index) / 10)))
        }
        XCTAssertFalse(drawing.addLevel(FibonacciLevel(ratio: 3)))
        drawing.style.reverse = true
        drawing.style.extendRight = true
        drawing.style.useLogCalculation = true
        drawing.levels[3].customText = "Strong support"
        let decoded = try JSONDecoder().decode(
            FibonacciRetracementDrawing.self, from: JSONEncoder().encode(drawing))
        XCTAssertEqual(decoded, drawing)
    }
}
