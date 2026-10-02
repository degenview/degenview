import SwiftUI
import XCTest

@testable import DegenView

final class PineDrawingGeometryTests: XCTestCase {
    func testDashPatternsAndArrowheadsPerLineStyle() {
        XCTAssertEqual(PineLineStyle.solid.dashPattern, [])
        XCTAssertEqual(PineLineStyle.dashed.dashPattern, [6, 4])
        XCTAssertEqual(PineLineStyle.dotted.dashPattern, [1, 3])
        XCTAssertEqual(PineLineStyle.arrowRight.dashPattern, [])
        XCTAssertTrue(PineLineStyle.arrowLeft.arrowheads == (true, false))
        XCTAssertTrue(PineLineStyle.arrowRight.arrowheads == (false, true))
        XCTAssertTrue(PineLineStyle.arrowBoth.arrowheads == (true, true))
        XCTAssertTrue(PineLineStyle.solid.arrowheads == (false, false))
    }

    func testArrowheadIsATriangleBehindTheTip() {
        let points = PineDrawingGeometry.arrowhead(
            tip: CGPoint(x: 10, y: 0), from: CGPoint(x: 0, y: 0), length: 4, halfWidth: 2)
        XCTAssertEqual(points, [CGPoint(x: 10, y: 0), CGPoint(x: 6, y: 2), CGPoint(x: 6, y: -2)])
        let diagonal = PineDrawingGeometry.arrowhead(
            tip: CGPoint(x: 3, y: 4), from: CGPoint(x: 0, y: 0), length: 5, halfWidth: 1)
        XCTAssertEqual(diagonal[0], CGPoint(x: 3, y: 4))
        XCTAssertEqual(diagonal[1].x, -0.8, accuracy: 1e-9)
        XCTAssertEqual(diagonal[1].y, 0.6 + 0.0, accuracy: 1e-9)
        XCTAssertEqual(
            PineDrawingGeometry.arrowhead(tip: .zero, from: .zero, length: 4, halfWidth: 2), [],
            "a zero-length line has no direction")
    }

    func testBoxTextPlacementFollowsTheAlignments() {
        let rect = CGRect(x: 10, y: 20, width: 100, height: 50)
        func placement(_ h: PineTextAlign, _ v: PineTextAlign) -> (CGPoint, UnitPoint) {
            let result = PineDrawingGeometry.boxTextPlacement(in: rect, horizontal: h, vertical: v, margin: 4)
            return (result.point, result.anchor)
        }
        XCTAssertTrue(placement(.center, .center) == (CGPoint(x: 60, y: 45), UnitPoint(x: 0.5, y: 0.5)))
        XCTAssertTrue(placement(.left, .top) == (CGPoint(x: 14, y: 24), UnitPoint(x: 0, y: 0)))
        XCTAssertTrue(placement(.right, .bottom) == (CGPoint(x: 106, y: 66), UnitPoint(x: 1, y: 1)))
        XCTAssertTrue(placement(.right, .center) == (CGPoint(x: 106, y: 45), UnitPoint(x: 1, y: 0.5)))
    }
}
