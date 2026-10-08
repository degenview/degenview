import SwiftUI
import XCTest

@testable import DegenView

final class WatchlistFlagShapeTests: XCTestCase {
    private let size = WatchlistMetrics.flagMarkSize
    private var rect: CGRect { CGRect(origin: .zero, size: size) }

    func testTheShapeFillsItsFrameExactly() {
        let path = WatchlistFlagShape().path(in: rect)
        XCTAssertEqual(path.boundingRect, rect)
        XCTAssertGreaterThan(size.height, size.width, "a bookmark's ribbon is the long way: taller than it is deep")
    }

    func testTheFlagIsTallerThanItWas() {
        XCTAssertGreaterThanOrEqual(size.height, 12)
    }

    func testTheLogoStartsInsideTheBookmarkSoItCoversItsFlatEnd() {
        // The mark is drawn from x = 0; the logo begins at the leading inset, a few points before the end.
        XCTAssertLessThan(WatchlistMetrics.leadingInset, size.width)
        XCTAssertGreaterThan(WatchlistMetrics.leadingInset, size.width / 2, "only a little of it is covered")
    }

    func testByDefaultTheNotchIsOnTheLeftAndTheFlatEndFacesTheLogo() {
        let path = WatchlistFlagShape().path(in: rect)
        let middle = size.height / 2
        // Flat right end, with its rounded corners cut a little.
        for y in [size.height * 0.25, middle, size.height * 0.75] {
            XCTAssertTrue(path.contains(CGPoint(x: size.width - 0.3, y: y)), "flat right edge at y \(y)")
        }
        XCTAssertFalse(path.contains(CGPoint(x: size.width - 0.05, y: 0.05)))
        // Tail tips at the left corners; the V between them is empty; solid right of the notch tip.
        XCTAssertTrue(path.contains(CGPoint(x: 0.4, y: 0.4)))
        XCTAssertTrue(path.contains(CGPoint(x: 0.4, y: size.height - 0.4)))
        XCTAssertFalse(path.contains(CGPoint(x: 0.4, y: middle)), "the V is cut into the left end")
        XCTAssertTrue(path.contains(CGPoint(x: 5, y: middle)), "right of the notch tip the middle is solid")
    }

    func testTheOtherOrientationPutsTheNotchOnTheRight() {
        let path = WatchlistFlagShape(tailPointsRight: true).path(in: rect)
        let middle = size.height / 2
        XCTAssertEqual(path.boundingRect, rect)
        for y in [size.height * 0.25, middle, size.height * 0.75] {
            XCTAssertTrue(path.contains(CGPoint(x: 0.3, y: y)), "flat left edge at y \(y)")
        }
        XCTAssertFalse(path.contains(CGPoint(x: size.width - 0.4, y: middle)), "the V is cut into the right end")
        XCTAssertTrue(path.contains(CGPoint(x: size.width - 0.4, y: 0.4)))
    }

    func testTheShapeFollowsItsOffsetFrame() {
        let moved = CGRect(origin: CGPoint(x: 20, y: 5), size: size)
        XCTAssertEqual(WatchlistFlagShape().path(in: moved).boundingRect, moved)
        XCTAssertEqual(WatchlistFlagShape(tailPointsRight: true).path(in: moved).boundingRect, moved)
    }

    @MainActor
    func testEveryFlagHasAFullColourSwatch() {
        for flag in WatchlistFlag.allCases {
            let swatch = flag.swatch
            XCTAssertFalse(swatch.isTemplate, "a template image would lose its colour in a menu")
            XCTAssertGreaterThan(swatch.size.width, 0)
        }
    }
}
