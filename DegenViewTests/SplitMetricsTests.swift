import XCTest

@testable import DegenView

final class SplitMetricsTests: XCTestCase {
    private func metrics(total: CGFloat = 1000, maxSecondary: CGFloat? = nil, defaultLength: CGFloat? = nil)
        -> SplitMetrics
    {
        SplitMetrics(
            total: total, minPrimary: 300, minSecondary: 200, maxSecondary: maxSecondary,
            defaultFraction: 0.4, defaultLength: defaultLength)
    }

    func testUnsetLengthUsesDefaultFraction() {
        XCTAssertEqual(metrics().secondaryLength(for: 0), 400)
    }

    func testDefaultLengthWinsOverFraction() {
        XCTAssertEqual(metrics(defaultLength: 240).secondaryLength(for: 0), 240)
    }

    func testStoredLengthIsKeptWithinLimits() {
        XCTAssertEqual(metrics().secondaryLength(for: 500), 500)
        XCTAssertEqual(metrics().secondaryLength(for: 50), 200, "never below the secondary minimum")
        XCTAssertEqual(metrics().secondaryLength(for: 900), 700, "always leaves the primary its minimum")
        XCTAssertEqual(metrics(maxSecondary: 320).secondaryLength(for: 600), 320)
    }

    func testTooSmallContainerLetsPrimaryMinimumYield() {
        // 400 < 300 + 200: the secondary pane gets what is left after the primary's minimum.
        XCTAssertEqual(metrics(total: 400).secondaryLength(for: 0), 100)
        XCTAssertEqual(metrics(total: 250).secondaryLength(for: 500), 0)
    }

    func testResizeMovesFromTheEffectiveLength() {
        // Never dragged: starts from the default (400), not from zero.
        XCTAssertEqual(metrics().resized(from: 0, by: 50), 450)
        XCTAssertEqual(metrics().resized(from: 450, by: -400), 200)
    }
}
