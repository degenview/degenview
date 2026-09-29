import XCTest

@testable import DegenView

final class PineCandleAggregatorTests: XCTestCase {
    private typealias F = PineExecutionFixtures
    private typealias Output = PineCandleAggregator.Output

    private func aggregator() -> PineCandleAggregator { PineCandleAggregator(barSeconds: F.barSeconds) }

    private func stream(_ index: Int, _ close: Double, closed: Bool = false) -> PineMarketUpdate {
        F.stream(F.bar(index, open: 100, close: close, closed: closed))
    }

    private func poll(_ index: Int, _ close: Double, closed: Bool = false) -> PineMarketUpdate {
        .init(bar: F.bar(index, open: 100, close: close, closed: closed), origin: .poll)
    }

    func testLifecycleOpenTicksCloseThenNextBar() {
        var a = aggregator()
        XCTAssertEqual(a.ingest(stream(0, 100)), [.tick(F.bar(0, open: 100, close: 100), isNew: true)])
        XCTAssertEqual(a.ingest(stream(0, 101)), [.tick(F.bar(0, open: 100, close: 101), isNew: false)])
        XCTAssertEqual(a.ingest(stream(0, 101)), [], "identical repeat")
        XCTAssertEqual(a.ingest(stream(0, 102, closed: true)), [.close(F.bar(0, open: 100, close: 102))])
        XCTAssertEqual(a.ingest(stream(0, 102, closed: true)), [], "repeat of the closing message")
        XCTAssertEqual(a.ingest(stream(0, 90)), [], "a late tick cannot reopen a closed bar")
        XCTAssertEqual(a.ingest(stream(1, 100)), [.tick(F.bar(1, open: 100, close: 100), isNew: true)])
    }

    func testTheNextBarClosesThePreviousOneWhenNoFlagArrives() {
        var a = aggregator()
        _ = a.ingest(stream(0, 100))
        _ = a.ingest(stream(0, 104))
        XCTAssertEqual(
            a.ingest(stream(1, 100)),
            [.close(F.bar(0, open: 100, close: 104)), .tick(F.bar(1, open: 100, close: 100), isNew: true)])
    }

    func testOlderBarsAreDropped() {
        var a = aggregator()
        _ = a.ingest(stream(5, 100))
        XCTAssertEqual(a.ingest(stream(4, 100)), [])
    }

    func testAGapAskForARebuild() {
        var a = aggregator()
        _ = a.ingest(stream(0, 100))
        XCTAssertEqual(a.ingest(stream(3, 100)), [.rebuild(.gap)])
    }

    func testABarOfTheWrongLengthAsksForARebuild() {
        var a = aggregator()
        _ = a.ingest(stream(0, 100))
        let half = F.bar(0, open: 100, close: 100)
        let odd = KlineData(
            openTime: half.openTime.addingTimeInterval(20), openPrice: 1, highPrice: 1, lowPrice: 1,
            closePrice: 1, volume: 1)
        XCTAssertEqual(a.ingest(F.stream(odd)), [.rebuild(.timeframeMismatch)])
    }

    func testUnknownBarLengthDisablesSpacingChecks() {
        var a = PineCandleAggregator()
        _ = a.ingest(stream(0, 100))
        XCTAssertEqual(a.ingest(stream(9, 100)).count, 2)
    }

    func testABarThatFirstArrivesClosedJustCloses() {
        var a = aggregator()
        XCTAssertEqual(a.ingest(stream(0, 100, closed: true)), [.close(F.bar(0, open: 100, close: 100))])
    }

    func testPollCannotOverwriteAStreamBar() {
        var a = aggregator()
        _ = a.ingest(stream(0, 105))
        XCTAssertEqual(a.ingest(poll(0, 99)), [], "a cached REST copy is older than the stream")
        XCTAssertEqual(a.ingest(stream(0, 106)).count, 1)
    }

    func testPollDrivesABarWhenNoStreamDoes() {
        var a = aggregator()
        XCTAssertEqual(a.ingest(poll(0, 100)).count, 1)
        XCTAssertEqual(a.ingest(poll(0, 101)), [.tick(F.bar(0, open: 100, close: 101), isNew: false)])
    }

    func testFinalPollValuesReplaceStreamValuesWhenTheBarIsKnownComplete() {
        var a = aggregator()
        _ = a.ingest(stream(0, 105))
        XCTAssertEqual(a.ingest(poll(0, 107, closed: true)), [.close(F.bar(0, open: 100, close: 107))])
    }

    func testCorrectedClosedBarAsksForARebuild() {
        var a = aggregator()
        _ = a.ingest(stream(0, 100, closed: true))
        XCTAssertEqual(a.ingest(poll(0, 100, closed: true)), [])
        XCTAssertEqual(a.ingest(poll(0, 101, closed: true)), [.rebuild(.correction)])
    }
}
