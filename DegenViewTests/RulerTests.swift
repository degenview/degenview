import XCTest

@testable import DegenView

final class RulerGeometryTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_790_078_400)

    private func time(_ hours: Double) -> Date { t0.addingTimeInterval(hours * 3600) }

    private func anchor(_ hours: Double, _ price: Double) -> TrendAnchor {
        TrendAnchor(date: time(hours), price: price)
    }

    private func opposite(_ corner: RulerCorner) -> RulerCorner {
        switch corner {
        case .topLeft: return .bottomRight
        case .topRight: return .bottomLeft
        case .bottomLeft: return .topRight
        case .bottomRight: return .topLeft
        }
    }

    /// Drawn bottom-left to top-right, the way an upward measurement usually is.
    private lazy var upward = RulerRect(start: anchor(0, 100), end: anchor(3, 120))
    /// Drawn top-right to bottom-left: right to left *and* downward.
    private lazy var downwardRightToLeft = RulerRect(start: anchor(3, 120), end: anchor(0, 100))

    func testCornersNameTheScreenPositionWhicheverWayTheBoxWasDrawn() {
        for rect in [upward, downwardRightToLeft] {
            XCTAssertEqual(rect.anchor(of: .topLeft), anchor(0, 120))
            XCTAssertEqual(rect.anchor(of: .topRight), anchor(3, 120))
            XCTAssertEqual(rect.anchor(of: .bottomLeft), anchor(0, 100))
            XCTAssertEqual(rect.anchor(of: .bottomRight), anchor(3, 100))
        }
    }

    func testResizingACornerLeavesTheOppositeCornerWhereItWas() {
        // Inside the box's span on both axes, so no drag crosses the opposite corner and
        // the corner names keep describing the same screen positions.
        let target = anchor(1.5, 110)
        for rect in [upward, downwardRightToLeft] {
            for corner in RulerCorner.allCases {
                let resized = rect.resized(corner: corner, to: target)
                XCTAssertEqual(
                    resized.anchor(of: opposite(corner)), rect.anchor(of: opposite(corner)),
                    "\(corner) drag moved the opposite corner")
                XCTAssertEqual(resized.id, rect.id)
            }
        }
    }

    func testResizingPutsTheDraggedCornerUnderThePointer() {
        let target = anchor(1.5, 110)
        for rect in [upward, downwardRightToLeft] {
            for corner in RulerCorner.allCases {
                XCTAssertEqual(rect.resized(corner: corner, to: target).anchor(of: corner), target)
            }
        }
    }

    func testResizingKeepsWhichAnchorIsTheStart() {
        // Dragging the end corner must not turn a measured rise into a fall.
        let resized = upward.resized(corner: .topRight, to: anchor(4, 150))
        XCTAssertEqual(resized.start, upward.start)
        XCTAssertEqual(resized.end, anchor(4, 150))
        XCTAssertTrue(resized.isUpward)
    }

    func testDraggingACornerPastTheOppositeOneFlipsTheBox() {
        let flipped = upward.resized(corner: .topRight, to: anchor(-2, 80))
        XCTAssertEqual(flipped.earliest, time(-2))
        XCTAssertEqual(flipped.high, 100)
        XCTAssertEqual(flipped.low, 80)
        XCTAssertFalse(flipped.isUpward)
    }

    func testTranslatingShiftsBothAnchorsAndKeepsTheShape() {
        let moved = upward.translated(by: 7200, price: -5)
        XCTAssertEqual(moved.start, anchor(2, 95))
        XCTAssertEqual(moved.end, anchor(5, 115))
        XCTAssertEqual(moved.id, upward.id)
    }

    func testFlatMeasurementCountsAsUpward() {
        XCTAssertTrue(RulerRect(start: anchor(0, 100), end: anchor(2, 100)).isUpward)
    }
}

final class RulerReadoutTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_790_078_400)

    /// Ten hourly points, so bar index == hours from `t0`.
    private var candles: [KlineData] {
        (0..<10).map { KlineData(time: t0.addingTimeInterval(Double($0) * 3600), price: 100 + Double($0)) }
    }

    /// The app localises the number, so the expectation goes through the same formatter
    /// rather than assuming a decimal point.
    private func percent(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(2)))
    }

    private func readout(_ startHour: Double, _ startPrice: Double, _ endHour: Double, _ endPrice: Double)
        -> RulerReadout
    {
        RulerReadout(
            rect: RulerRect(
                start: TrendAnchor(date: t0.addingTimeInterval(startHour * 3600), price: startPrice),
                end: TrendAnchor(date: t0.addingTimeInterval(endHour * 3600), price: endPrice)),
            points: candles)
    }

    func testUpwardMove() {
        let r = readout(2, 100, 5, 104)
        XCTAssertEqual(r.percent, 4, accuracy: 0.0001)
        XCTAssertEqual(r.priceDelta, 4, accuracy: 0.0001)
        XCTAssertTrue(r.isUpward)
        XCTAssertEqual(r.percentText, "+\(percent(4))%")
        XCTAssertTrue(r.priceText(decimalPlaces: 2, scale: .currency).hasPrefix("+"))
    }

    func testDownwardMove() {
        let r = readout(5, 100, 2, 90)
        XCTAssertEqual(r.percent, -10, accuracy: 0.0001)
        XCTAssertFalse(r.isUpward)
        XCTAssertEqual(r.percentText, "-\(percent(10))%")
        XCTAssertTrue(r.priceText(decimalPlaces: 2, scale: .currency).hasPrefix("-"))
    }

    func testFlatMoveReadsAsPlusZero() {
        let r = readout(1, 100, 3, 100)
        XCTAssertTrue(r.isUpward)
        XCTAssertEqual(r.percentText, "+\(percent(0))%")
    }

    func testBarCountIsInclusiveOfBothEnds() {
        XCTAssertEqual(readout(2, 100, 5, 104).bars, 4)
        XCTAssertEqual(readout(2, 100, 3, 104).bars, 2)
        XCTAssertEqual(readout(2, 100, 2, 104).bars, 1)
        XCTAssertEqual(readout(2, 100, 2, 104).barsText, "1 bar")
        XCTAssertEqual(readout(2, 100, 5, 104).barsText, "4 bars")
    }

    func testRightToLeftSpansAreStillPositive() {
        let r = readout(5, 100, 2, 104)
        XCTAssertEqual(r.bars, 4)
        XCTAssertEqual(r.span, 3 * 3600, accuracy: 0.001)
        XCTAssertFalse(r.durationText.hasPrefix("-"))
        XCTAssertEqual(r.coverageText, "4 bars · \(r.durationText)")
    }

    func testSpanUnderAMinuteLeavesDurationOut() {
        let r = RulerReadout(
            rect: RulerRect(
                start: TrendAnchor(date: t0, price: 100),
                end: TrendAnchor(date: t0.addingTimeInterval(20), price: 101)),
            points: candles)
        XCTAssertEqual(r.durationText, "")
        XCTAssertEqual(r.coverageText, r.barsText)
    }

    func testZeroStartPriceNeverProducesNaN() {
        let r = readout(1, 0, 3, 5)
        XCTAssertEqual(r.percent, 0)
        XCTAssertEqual(r.percentText, "+\(percent(0))%")
    }

    func testProbabilityScaleKeepsTheSignOutsideTheClampedFormatter() {
        let r = RulerReadout(
            rect: RulerRect(
                start: TrendAnchor(date: t0, price: 0.6),
                end: TrendAnchor(date: t0.addingTimeInterval(3600), price: 0.4)),
            points: candles)
        let text = r.priceText(decimalPlaces: nil, scale: .probability)
        XCTAssertTrue(text.hasPrefix("-"))
        XCTAssertNotEqual(text, "-0.0%")
    }
}

@MainActor
final class RulerChartViewModelTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_790_078_400)
    private let size = CGSize(width: 500, height: 300)

    private final class EmptySource: TickerDataSource {
        let type = DataSourceType.binance
        func fetchKlines(symbol: String, interval: String, limit: Int) async throws -> [KlineData] { [] }
        func searchTickers(query: String) async throws -> [TickerSearchResult] { [] }
    }

    private func makeChart() -> ChartViewModel {
        let chart = ChartViewModel(ticker: "BTC", api: EmptySource())
        chart.klineData = (0..<20).map {
            KlineData(time: t0.addingTimeInterval(Double($0) * 3600), price: 100 + Double($0))
        }
        return chart
    }

    private func anchor(_ hours: Double, _ price: Double) -> TrendAnchor {
        TrendAnchor(date: t0.addingTimeInterval(hours * 3600), price: price)
    }

    /// Draws a ruler from bar 4 @ 105 to bar 12 @ 115 and returns its on-screen box.
    private func drawRuler(on chart: ChartViewModel) -> (plot: ChartPlot, box: CGRect) {
        let plot = chart.plot(in: size)
        let start = anchor(4, 105)
        let end = anchor(12, 115)
        chart.beginRulerDraft(at: start)
        XCTAssertTrue(chart.commitRulerDraft(at: end, in: plot))

        let points = chart.visibleKlines
        let slot = plot.slotWidth(forCount: points.count)
        let from = plot.position(of: start, points: points, slotWidth: slot)
        let to = plot.position(of: end, points: points, slotWidth: slot)
        let box = CGRect(
            x: min(from.x, to.x), y: min(from.y, to.y), width: abs(to.x - from.x), height: abs(to.y - from.y))
        return (plot, box)
    }

    func testCommitSelectsTheNewRulerAndClosesTheDraft() {
        let chart = makeChart()
        _ = drawRuler(on: chart)

        XCTAssertEqual(chart.rulers.count, 1)
        XCTAssertEqual(chart.selectedRulerID, chart.rulers.first?.id)
        XCTAssertFalse(chart.hasRulerDraft)
    }

    func testCommitOnTheFirstCornerIsRejectedAndKeepsTheDraftOpen() {
        let chart = makeChart()
        let plot = chart.plot(in: size)
        chart.beginRulerDraft(at: anchor(4, 105))

        XCTAssertFalse(chart.commitRulerDraft(at: anchor(4, 105), in: plot))
        XCTAssertTrue(chart.hasRulerDraft)
        XCTAssertTrue(chart.rulers.isEmpty)
    }

    func testSeveralRulersCanShareAChart() {
        let chart = makeChart()
        _ = drawRuler(on: chart)
        _ = drawRuler(on: chart)
        XCTAssertEqual(chart.rulers.count, 2)
    }

    func testHitTestingFindsCornersAndEdgesButNotTheInterior() {
        let chart = makeChart()
        let (plot, box) = drawRuler(on: chart)
        let id = chart.rulers[0].id

        XCTAssertEqual(
            chart.rulerHit(at: CGPoint(x: box.minX + 1, y: box.minY + 1), in: plot),
            RulerHit(id: id, part: .corner(.topLeft)))
        XCTAssertEqual(
            chart.rulerHit(at: CGPoint(x: box.maxX, y: box.maxY), in: plot),
            RulerHit(id: id, part: .corner(.bottomRight)))
        XCTAssertEqual(
            chart.rulerHit(at: CGPoint(x: box.midX, y: box.minY + 1), in: plot),
            RulerHit(id: id, part: .edge))
        XCTAssertEqual(
            chart.rulerHit(at: CGPoint(x: box.minX - 2, y: box.midY), in: plot),
            RulerHit(id: id, part: .edge))
        XCTAssertNil(chart.rulerHit(at: CGPoint(x: box.midX, y: box.midY), in: plot))
        XCTAssertNil(chart.rulerHit(at: CGPoint(x: box.maxX + 40, y: box.maxY + 40), in: plot))
    }

    func testMovingACornerRewritesTheStoredRulerFromTheOriginal() {
        let chart = makeChart()
        _ = drawRuler(on: chart)
        let original = chart.rulers[0]

        chart.moveRulerCorner(original: original, corner: .topRight, to: anchor(15, 118))
        XCTAssertEqual(chart.rulers[0].end, anchor(15, 118))
        XCTAssertEqual(chart.rulers[0].start, original.start)

        // A later drag event starts again from the original, not from the previous event.
        chart.moveRulerCorner(original: original, corner: .topRight, to: anchor(10, 112))
        XCTAssertEqual(chart.rulers[0].end, anchor(10, 112))
    }

    func testTranslatingMovesTheWholeRuler() {
        let chart = makeChart()
        _ = drawRuler(on: chart)
        let original = chart.rulers[0]

        chart.translateRuler(original: original, from: anchor(6, 108), to: anchor(8, 106))
        XCTAssertEqual(chart.rulers[0].start, anchor(6, 103))
        XCTAssertEqual(chart.rulers[0].end, anchor(14, 113))
    }

    func testRemoveSelectedOnlyRemovesTheSelectedRuler() {
        let chart = makeChart()
        _ = drawRuler(on: chart)
        let first = chart.rulers[0].id
        _ = drawRuler(on: chart)
        let second = chart.rulers[1].id

        chart.selectRuler(first)
        XCTAssertTrue(chart.removeSelectedRuler())
        XCTAssertEqual(chart.rulers.map(\.id), [second])
        XCTAssertNil(chart.selectedRulerID)
        XCTAssertFalse(chart.removeSelectedRuler())
    }

    func testClearRulersResetsSelectionAndHover() {
        let chart = makeChart()
        _ = drawRuler(on: chart)
        chart.setRulerHover(RulerHit(id: chart.rulers[0].id, part: .edge))

        XCTAssertTrue(chart.clearRulers())
        XCTAssertTrue(chart.rulers.isEmpty)
        XCTAssertNil(chart.selectedRulerID)
        XCTAssertNil(chart.hoveredRuler)
        XCTAssertFalse(chart.clearRulers())
    }

    func testBeginningANewDraftDeselects() {
        let chart = makeChart()
        _ = drawRuler(on: chart)
        chart.beginRulerDraft(at: anchor(2, 102))
        XCTAssertNil(chart.selectedRulerID)
        XCTAssertEqual(chart.rulerOverlay.rects.count, 1)
        XCTAssertNotNil(chart.rulerOverlay.draft)
    }
}
