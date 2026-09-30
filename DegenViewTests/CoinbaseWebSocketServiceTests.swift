import XCTest

@testable import DegenView

final class CoinbaseWebSocketServiceTests: XCTestCase {
    final class OpenRecorder: @unchecked Sendable {
        private let lock = NSLock()
        private var storage: [URL] = []
        func append(_ url: URL) { lock.withLock { storage.append(url) } }
        var count: Int { lock.withLock { storage.count } }
    }

    private func tickFrame(product: String = "BTC-USD", price: String = "84080.23", size: String = "0.5", trade: Int) -> String {
        """
        {"type":"ticker","sequence":1,"product_id":"\(product)","price":"\(price)","open_24h":"83047.65",\
        "volume_24h":"6179.8","low_24h":"82911.86","high_24h":"85613.72","best_bid":"84080.22",\
        "best_ask":"84080.23","side":"buy","time":"2026-09-30T17:28:25.629380Z","trade_id":\(trade),\
        "last_size":"\(size)"}
        """
    }

    func testStaleReconnectCannotReplaceNewConnection() async {
        let recorder = OpenRecorder()
        let service = CoinbaseWebSocketService(
            socketOpenObserver: recorder.append,
            reconnectSleep: { _ in try await Task.sleep(for: .milliseconds(60)) }
        )
        service.connect(products: ["BTC-USD"]) { _ in }
        let oldGeneration = service.currentConnectionGeneration
        service.connectionDidFail(generation: oldGeneration)

        service.connect(products: ["ETH-USD"]) { _ in }
        try? await Task.sleep(for: .milliseconds(100))

        XCTAssertEqual(recorder.count, 2, "The old delayed retry must not open a third socket")
    }

    func testDisconnectCancelsPendingReconnect() async {
        let recorder = OpenRecorder()
        let service = CoinbaseWebSocketService(
            socketOpenObserver: recorder.append,
            reconnectSleep: { _ in try await Task.sleep(for: .milliseconds(60)) }
        )
        service.connect(products: ["BTC-USD"]) { _ in }
        service.connectionDidFail(generation: service.currentConnectionGeneration)
        service.disconnect()
        try? await Task.sleep(for: .milliseconds(100))

        XCTAssertEqual(recorder.count, 1)
    }

    func testOpensTheExchangeFeed() {
        let recorder = OpenRecorder()
        var opened: URL?
        let service = CoinbaseWebSocketService(socketOpenObserver: { opened = $0; recorder.append($0) })
        service.connect(products: ["BTC-USD"]) { _ in }

        XCTAssertEqual(opened?.absoluteString, "wss://ws-feed.exchange.coinbase.com")
    }

    func testSubscribeMessageAsksForTickerAndHeartbeat() throws {
        let text = CoinbaseWebSocketService.subscribeMessage(products: ["BTC-USD", "ETH-USD"])
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any])

        XCTAssertEqual(payload["type"] as? String, "subscribe")
        XCTAssertEqual(payload["product_ids"] as? [String], ["BTC-USD", "ETH-USD"])
        XCTAssertEqual(payload["channels"] as? [String], ["ticker", "heartbeat"])
    }

    func testTickerFrameBecomesATick() throws {
        var ticks: [CoinbaseTick] = []
        let service = CoinbaseWebSocketService(socketOpenObserver: { _ in })
        service.connect(products: ["BTC-USD"]) { ticks.append($0) }

        service.handleMessage(tickFrame(price: "84080.23", size: "0.5", trade: 10))

        let tick = try XCTUnwrap(ticks.first)
        XCTAssertEqual(tick.productID, "BTC-USD")
        XCTAssertEqual(tick.price, 84_080.23, accuracy: 1e-9)
        XCTAssertEqual(tick.size, 0.5, accuracy: 1e-9)
        XCTAssertEqual(tick.tradeID, 10)
        XCTAssertEqual(tick.time.timeIntervalSince1970, 1_790_789_305.629, accuracy: 0.001)
    }

    func testNonTickerFramesAreIgnored() {
        var ticks: [CoinbaseTick] = []
        let service = CoinbaseWebSocketService(socketOpenObserver: { _ in })
        service.connect(products: ["BTC-USD"]) { ticks.append($0) }

        service.handleMessage(#"{"type":"subscriptions","channels":[{"name":"ticker","product_ids":["BTC-USD"]}]}"#)
        service.handleMessage(#"{"type":"heartbeat","sequence":1,"last_trade_id":5,"product_id":"BTC-USD","time":"2026-09-30T17:28:27.000000Z"}"#)
        service.handleMessage(#"{"type":"error","message":"Failed to subscribe","reason":"candles is not a valid channel"}"#)
        service.handleMessage("not json")

        XCTAssertTrue(ticks.isEmpty)
    }

    func testReplayedTradeIsNotDeliveredTwice() {
        var ticks: [CoinbaseTick] = []
        let service = CoinbaseWebSocketService(socketOpenObserver: { _ in })
        service.connect(products: ["BTC-USD", "ETH-USD"]) { ticks.append($0) }

        service.handleMessage(tickFrame(trade: 10))
        service.handleMessage(tickFrame(trade: 10))  // what a reconnect resends
        service.handleMessage(tickFrame(trade: 9))  // out of order
        service.handleMessage(tickFrame(trade: 11))
        service.handleMessage(tickFrame(product: "ETH-USD", trade: 10))  // ids are per product

        XCTAssertEqual(ticks.map(\.tradeID), [10, 11, 10])
    }
}
