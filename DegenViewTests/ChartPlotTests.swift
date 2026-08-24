import XCTest
import SwiftUI
@testable import DegenView

final class ChartPlotTests: XCTestCase {
    func testFlatZeroAndNegativeRangesRemainOrderedAndFinite() {
        for price in [0.0, -1.0, 100.0] {
            let range = ChartPlot.priceRange(for: [KlineData(time: .distantPast, price: price)], padding: 0.05)
            XCTAssertLessThan(range.min, range.max)
            XCTAssertTrue(range.min.isFinite)
            XCTAssertTrue(range.max.isFinite)
        }
    }

    func testTinyCanvasNeverCreatesNegativePlotDimensions() {
        let rect = ChartPlot.rect(in: CGSize(width: 1, height: 1), insets: EdgeInsets(top: 10, leading: 10, bottom: 10, trailing: 10))
        XCTAssertEqual(rect.width, 0)
        XCTAssertEqual(rect.height, 0)
    }

    func testCoordinateConversionsRoundTrip() {
        let plot = ChartPlot.make(points: [KlineData(time: .distantPast, price: 10), KlineData(time: .now, price: 20)], size: CGSize(width: 400, height: 240), yZoom: 1, scale: .currency, yAxisDecimalPlaces: nil)
        let slot = plot.slotWidth(forCount: 2)
        XCTAssertEqual(plot.fractionalIndex(forX: plot.x(forFractionalIndex: 0.75, slotWidth: slot), slotWidth: slot), 0.75, accuracy: 0.0001)
        XCTAssertEqual(plot.price(forY: plot.y(for: 15)), 15, accuracy: 0.0001)
    }
}
