import XCTest

@testable import DegenView

final class BinanceWebSocketServiceTests: XCTestCase {
    final class OpenRecorder: @unchecked Sendable {
        private let lock = NSLock()
        private var storage: [URL] = []
        func append(_ url: URL) { lock.withLock { storage.append(url) } }
        var count: Int { lock.withLock { storage.count } }
    }

    func testStaleReconnectCannotReplaceNewConnection() async {
        let recorder = OpenRecorder()
        let service = BinanceWebSocketService(
            socketOpenObserver: recorder.append,
            reconnectSleep: { _ in try await Task.sleep(for: .milliseconds(60)) }
        )
        service.connect(symbols: ["BTCUSDT"], interval: "1m") { _, _ in }
        let oldGeneration = service.currentConnectionGeneration
        service.connectionDidFail(generation: oldGeneration)

        service.connect(symbols: ["ETHUSDT"], interval: "5m") { _, _ in }
        try? await Task.sleep(for: .milliseconds(100))

        XCTAssertEqual(recorder.count, 2, "The old delayed retry must not open a third socket")
    }

    func testDisconnectCancelsPendingReconnect() async {
        let recorder = OpenRecorder()
        let service = BinanceWebSocketService(
            socketOpenObserver: recorder.append,
            reconnectSleep: { _ in try await Task.sleep(for: .milliseconds(60)) }
        )
        service.connect(symbols: ["BTCUSDT"], interval: "1m") { _, _ in }
        service.connectionDidFail(generation: service.currentConnectionGeneration)
        service.disconnect()
        try? await Task.sleep(for: .milliseconds(100))

        XCTAssertEqual(recorder.count, 1)
    }
}
