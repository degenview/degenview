import XCTest

@testable import DegenView

final class PineLabelGeometryTests: XCTestCase {
    private let anchor = CGPoint(x: 100, y: 50)
    private let size = CGSize(width: 40, height: 20)
    private let pointer: CGFloat = 5

    private func layout(_ style: PineLabelStyle) -> PineLabelGeometry.Layout {
        PineLabelGeometry.layout(style, anchor: anchor, size: size, pointer: pointer)
    }

    func testBubblesWithAPointerSitOnTheirSideOfTheAnchor() {
        let down = layout(.labelDown)
        XCTAssertEqual(down.body, CGRect(x: 80, y: 25, width: 40, height: 20), "above the anchor")
        XCTAssertEqual(down.shape, .bubble(pointer: [CGPoint(x: 95, y: 45), anchor, CGPoint(x: 105, y: 45)]))
        XCTAssertEqual(layout(.labelUp).body, CGRect(x: 80, y: 55, width: 40, height: 20), "below")
        XCTAssertEqual(layout(.labelLeft).body, CGRect(x: 105, y: 40, width: 40, height: 20), "right of it")
        XCTAssertEqual(layout(.labelRight).body, CGRect(x: 55, y: 40, width: 40, height: 20), "left of it")
    }

    func testCornerBubblesHaveTheAnchorAtTheNamedCorner() {
        XCTAssertEqual(layout(.labelLowerLeft).body, CGRect(x: 100, y: 30, width: 40, height: 20))
        XCTAssertEqual(layout(.labelLowerRight).body, CGRect(x: 60, y: 30, width: 40, height: 20))
        XCTAssertEqual(layout(.labelUpperLeft).body, CGRect(x: 100, y: 50, width: 40, height: 20))
        XCTAssertEqual(layout(.labelUpperRight).body, CGRect(x: 60, y: 50, width: 40, height: 20))
        let pointer = layout(.labelLowerLeft).shape
        XCTAssertEqual(pointer, .bubble(pointer: [CGPoint(x: 105, y: 50), anchor, CGPoint(x: 100, y: 45)]))
    }

    func testCenteredStylesSurroundTheAnchor() {
        let centre = CGRect(x: 80, y: 40, width: 40, height: 20)
        XCTAssertEqual(layout(.labelCenter).body, centre)
        XCTAssertEqual(layout(.labelCenter).shape, .bubble(pointer: []))
        XCTAssertEqual(layout(.none).shape, .textOnly)
        XCTAssertEqual(layout(.textOutline).shape, .textOnly)
        XCTAssertEqual(layout(.square).body, centre)
        XCTAssertEqual(layout(.square).shape, .rectangle)
    }

    func testCircleHoldsTheTextAndDiamondIsTwiceAsBig() {
        let circle = layout(.circle)
        XCTAssertEqual(circle.shape, .ellipse)
        XCTAssertEqual(circle.body.midX, anchor.x, accuracy: 1e-9)
        XCTAssertEqual(circle.body.midY, anchor.y, accuracy: 1e-9)
        XCTAssertEqual(circle.body.width, circle.body.height)
        XCTAssertGreaterThanOrEqual(circle.body.width, (40 * 40 + 20 * 20).squareRoot() - 1e-9)
        XCTAssertEqual(layout(.diamond).body, CGRect(x: 60, y: 30, width: 80, height: 40))
        XCTAssertEqual(layout(.diamond).shape, .diamond)
    }

    func testMarkerStylesPutAGlyphOnTheAnchorAndTheTextBesideIt() {
        let glyph = CGRect(x: 90, y: 40, width: 20, height: 20)
        for (style, marker) in [
            (PineLabelStyle.xcross, PineLabelGeometry.Marker.xcross), (.cross, .cross), (.flag, .flag),
            (.triangledown, .triangleDown), (.arrowdown, .arrowDown),
        ] {
            let result = layout(style)
            XCTAssertEqual(result.shape, .marker(marker), "\(style)")
            XCTAssertEqual(result.glyph, glyph, "\(style)")
            XCTAssertEqual(result.body, CGRect(x: 80, y: 20, width: 40, height: 20), "text above \(style)")
        }
        for (style, marker) in [
            (PineLabelStyle.triangleup, PineLabelGeometry.Marker.triangleUp), (.arrowup, .arrowUp),
        ] {
            let result = layout(style)
            XCTAssertEqual(result.shape, .marker(marker))
            XCTAssertEqual(result.body, CGRect(x: 80, y: 60, width: 40, height: 20), "text below \(style)")
        }
    }

    func testEveryStyleHasALayoutAndAPineName() {
        for style in [PineLabelStyle.none] + [PineLabelStyle.circle, .square, .diamond, .cross, .xcross, .flag] {
            XCTAssertEqual(PineLabelStyle(pineName: "label.style_\(style.rawValue)"), style)
            XCTAssertFalse(layout(style).body.isEmpty)
        }
        XCTAssertEqual(PineLabelStyle(pineName: "label.style_label_lower_left"), .labelLowerLeft)
        XCTAssertEqual(PineLabelStyle(pineName: "label.style_text_outline"), .textOutline)
    }
}
