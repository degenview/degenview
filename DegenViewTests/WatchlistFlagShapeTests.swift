import SwiftUI
import XCTest

@testable import DegenView

final class WatchlistFlagShapeTests: XCTestCase {
    private let rect = CGRect(x: 0, y: 0, width: 10, height: 8)

    func testTheShapeFillsItsFrameExactly() {
        let path = WatchlistFlagShape().path(in: rect)
        XCTAssertEqual(path.boundingRect, rect)
        XCTAssertEqual(WatchlistMetrics.flagMarkSize, CGSize(width: 10, height: 8))
    }

    func testTheFlatEndIsOnTheLeftAndStaysInsideItsCorners() {
        let path = WatchlistFlagShape().path(in: rect)
        for y in [2.0, 4.0, 6.0] { XCTAssertTrue(path.contains(CGPoint(x: 0.3, y: y)), "flat left edge at y \(y)") }
        // The rounded corners are cut a little; the straight run between them is not.
        XCTAssertFalse(path.contains(CGPoint(x: 0.05, y: 0.05)))
    }

    func testTheTailIsNotchedAndTheNotchPointsAtTheLeft() {
        let path = WatchlistFlagShape().path(in: rect)
        // Just inside the tail tips (top and bottom right corners) is filled; the notch between them is empty.
        XCTAssertTrue(path.contains(CGPoint(x: 9.5, y: 0.6)))
        XCTAssertTrue(path.contains(CGPoint(x: 9.5, y: 7.4)))
        XCTAssertFalse(path.contains(CGPoint(x: 9.5, y: 4)), "the V is cut into the right end")
        XCTAssertTrue(path.contains(CGPoint(x: 6, y: 4)), "left of the notch tip the middle is solid")
    }

    func testTheMirroredShapeHasItsFlatEndOnTheRight() {
        let path = WatchlistFlagShape(tailPointsRight: false).path(in: rect)
        XCTAssertEqual(path.boundingRect, rect)
        XCTAssertTrue(path.contains(CGPoint(x: 9.7, y: 4)))
        XCTAssertFalse(path.contains(CGPoint(x: 0.5, y: 4)), "the notch is now on the left")
    }

    func testTheShapeFollowsItsOffsetFrame() {
        let moved = CGRect(x: 20, y: 5, width: 10, height: 8)
        XCTAssertEqual(WatchlistFlagShape().path(in: moved).boundingRect, moved)
        XCTAssertEqual(WatchlistFlagShape(tailPointsRight: false).path(in: moved).boundingRect, moved)
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
