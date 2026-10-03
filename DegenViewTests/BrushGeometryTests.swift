import SwiftUI
import XCTest

@testable import DegenView

final class BrushGeometryTests: XCTestCase {
    private func line(_ count: Int) -> [CGPoint] {
        (0..<count).map { CGPoint(x: CGFloat($0) * 3, y: CGFloat($0) * 1.5) }
    }

    func testSimplifyDropsPointsOnAStraightRun() {
        XCTAssertEqual(BrushGeometry.simplify(line(50), tolerance: 0.75), [0, 49])
    }

    func testSimplifyKeepsASharpCorner() {
        let v = [
            CGPoint(x: 0, y: 0), CGPoint(x: 25, y: 50), CGPoint(x: 50, y: 100), CGPoint(x: 75, y: 50),
            CGPoint(x: 100, y: 0),
        ]
        XCTAssertEqual(BrushGeometry.simplify(v, tolerance: 0.75), [0, 2, 4])
    }

    func testSimplifyKeepsEndsAndShortInputs() {
        XCTAssertEqual(BrushGeometry.simplify([], tolerance: 1), [])
        XCTAssertEqual(BrushGeometry.simplify([.zero], tolerance: 1), [0])
        XCTAssertEqual(BrushGeometry.simplify([.zero, CGPoint(x: 1, y: 1)], tolerance: 1), [0, 1])
    }

    func testSimplifyIsDeterministicAndKeepsACircle() {
        let circle = (0..<200).map { index -> CGPoint in
            let angle = Double(index) / 199 * 2 * .pi
            return CGPoint(x: 100 + 40 * cos(angle), y: 100 + 40 * sin(angle))
        }
        let first = BrushGeometry.simplify(circle, tolerance: 0.75)
        XCTAssertEqual(first, BrushGeometry.simplify(circle, tolerance: 0.75))
        XCTAssertGreaterThan(first.count, 12)
        XCTAssertLessThan(first.count, circle.count)
        // Every dropped point stays within tolerance of the simplified outline.
        let kept = first.map { circle[$0] }
        for point in circle {
            let nearest =
                (1..<kept.count).map {
                    BrushGeometry.distance(from: point, toSegmentFrom: kept[$0 - 1], to: kept[$0])
                }.min() ?? .infinity
            XCTAssertLessThanOrEqual(nearest, 0.75 + 0.001)
        }
    }

    func testDistanceToSegment() {
        let a = CGPoint(x: 0, y: 0)
        let b = CGPoint(x: 10, y: 0)
        XCTAssertEqual(BrushGeometry.distance(from: CGPoint(x: 5, y: 3), toSegmentFrom: a, to: b), 3, accuracy: 0.0001)
        XCTAssertEqual(BrushGeometry.distance(from: CGPoint(x: 13, y: 4), toSegmentFrom: a, to: b), 5, accuracy: 0.0001)
        XCTAssertEqual(BrushGeometry.distance(from: CGPoint(x: 3, y: 4), toSegmentFrom: a, to: a), 5, accuracy: 0.0001)
    }

    func testHitUsesStrokeWidthPlusTolerance() {
        let stroke = [CGPoint(x: 0, y: 0), CGPoint(x: 100, y: 0)]
        XCTAssertTrue(BrushGeometry.hit(point: CGPoint(x: 50, y: 9), in: stroke, halfWidth: 2, tolerance: 8))
        XCTAssertFalse(BrushGeometry.hit(point: CGPoint(x: 50, y: 11), in: stroke, halfWidth: 2, tolerance: 8))
        // Inside the bounding box of a diagonal, but far from the stroke itself.
        let diagonal = [CGPoint(x: 0, y: 0), CGPoint(x: 100, y: 100)]
        XCTAssertFalse(BrushGeometry.hit(point: CGPoint(x: 90, y: 10), in: diagonal, halfWidth: 1, tolerance: 8))
        XCTAssertFalse(BrushGeometry.hit(point: CGPoint(x: 500, y: 500), in: diagonal, halfWidth: 1, tolerance: 8))
    }

    func testHitOnADot() {
        let dot = [CGPoint(x: 20, y: 20)]
        XCTAssertTrue(BrushGeometry.hit(point: CGPoint(x: 25, y: 20), in: dot, halfWidth: 2, tolerance: 8))
        XCTAssertFalse(BrushGeometry.hit(point: CGPoint(x: 40, y: 20), in: dot, halfWidth: 2, tolerance: 8))
    }

    func testSmoothedPathNeverLeavesThePolylineHull() {
        let zigzag = (0..<12).map { CGPoint(x: CGFloat($0) * 10, y: $0 % 2 == 0 ? 0 : 40) }
        let box = BrushGeometry.bounds(of: zigzag)!.insetBy(dx: -0.001, dy: -0.001)
        XCTAssertTrue(box.contains(BrushGeometry.smoothedPath(zigzag).boundingRect))

        let circle = (0..<60).map { index -> CGPoint in
            let angle = Double(index) / 59 * 2 * .pi
            return CGPoint(x: 100 + 40 * cos(angle), y: 100 + 40 * sin(angle))
        }
        let circleBox = BrushGeometry.bounds(of: circle)!.insetBy(dx: -0.001, dy: -0.001)
        XCTAssertTrue(circleBox.contains(BrushGeometry.smoothedPath(circle).boundingRect))
    }

    func testSharpVertexStaysSharpAndGentleOneIsRounded() {
        func curveCount(_ path: Path) -> Int {
            var count = 0
            path.forEach { element in
                if case .quadCurve = element { count += 1 }
            }
            return count
        }

        let v = [CGPoint(x: 0, y: 0), CGPoint(x: 50, y: 100), CGPoint(x: 100, y: 0)]
        let sharp = BrushGeometry.smoothedPath(v)
        XCTAssertEqual(curveCount(sharp), 0)
        XCTAssertEqual(sharp.boundingRect.maxY, 100, accuracy: 0.0001)

        let bend = [CGPoint(x: 0, y: 0), CGPoint(x: 50, y: 0), CGPoint(x: 100, y: 20)]
        XCTAssertEqual(curveCount(BrushGeometry.smoothedPath(bend)), 1)
    }

    func testSmoothedPathHandlesDegenerateInput() {
        XCTAssertTrue(BrushGeometry.smoothedPath([]).isEmpty)
        XCTAssertFalse(BrushGeometry.smoothedPath([.zero, .zero, CGPoint(x: 5, y: 5)]).isEmpty)
    }

    func testTurnAngle() {
        let straight = BrushGeometry.turnAngle(.zero, CGPoint(x: 1, y: 0), CGPoint(x: 2, y: 0))
        XCTAssertEqual(straight, 0, accuracy: 0.0001)
        let uTurn = BrushGeometry.turnAngle(.zero, CGPoint(x: 1, y: 0), .zero)
        XCTAssertEqual(uTurn, .pi, accuracy: 0.0001)
    }
}
