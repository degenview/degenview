import XCTest

@testable import DegenView

final class PineAlertDispatcherTests: XCTestCase {
    private func notification(_ message: String = "hello") -> PineAlertNotification {
        PineAlertNotification(
            subscriptionID: UUID(), scriptName: "S", chartID: UUID(), symbolKey: "binance:BTC", timeframe: "1H",
            barTime: Date(timeIntervalSince1970: 0), message: message, frequency: .oncePerBar, isConfirmed: false)
    }

    func testEveryChannelReceivesTheNotification() async {
        let first = RecordingPineAlertChannel()
        let second = RecordingPineAlertChannel()
        let dispatcher = PineAlertDispatcher(channels: [first, second])
        let item = notification()
        await dispatcher.dispatch(item)
        XCTAssertEqual(first.delivered, [item])
        XCTAssertEqual(second.delivered, [item])
    }

    func testFailingChannelDoesNotBlockTheOthers() async {
        let recording = RecordingPineAlertChannel()
        let dispatcher = PineAlertDispatcher(channels: [FailingPineAlertChannel(), recording])
        let item = notification()
        await dispatcher.dispatch(item)  // does not throw
        XCTAssertEqual(recording.delivered, [item])
    }

    func testNotificationDescribesItsScriptAndMarket() {
        let item = notification("Long @ 101")
        XCTAssertEqual(item.symbol, "BTC")
        XCTAssertEqual(item.title, "S · BTC 1H")
    }
}
