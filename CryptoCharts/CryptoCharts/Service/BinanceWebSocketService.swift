import Foundation

// MARK: - WebSocket message models

private struct WSKlineMessage: Decodable {
    let stream: String
    let data: WSKlineEvent
}

private struct WSKlineEvent: Decodable {
    let k: WSKlinePayload
}

private struct WSKlinePayload: Decodable {
    let t: Int64   // Kline start time (ms)
    let o: String  // Open
    let h: String  // High
    let l: String  // Low
    let c: String  // Close
    let v: String  // Volume
    let x: Bool    // Is this kline closed? (unused — kept as decode placeholder)

    func toKlineData() -> KlineData? {
        guard let open = Double(o),
              let high = Double(h),
              let low = Double(l),
              let close = Double(c),
              let volume = Double(v) else { return nil }
        return KlineData(
            openTime: Date(timeIntervalSince1970: Double(t) / 1000.0),
            openPrice: open,
            highPrice: high,
            lowPrice: low,
            closePrice: close,
            volume: volume
        )
    }
}

// MARK: - WebSocket service

/// Manages a single Binance combined-stream WebSocket connection.
/// All callbacks fire on the main thread (URLSession delegate queue = .main).
final class BinanceWebSocketService {

    private var webSocket: URLSessionWebSocketTask?
    private var session: URLSession?
    private var onUpdate: ((String, KlineData) -> Void)?
    private var symbols: [String] = []
    private var interval: String = ""
    private var reconnectCount = 0
    private var isShuttingDown = false

    /// Open a combined stream for the given symbols and interval.
    /// - Parameters:
    ///   - symbols: e.g. `["BTCUSDT", "ETHUSDT"]`
    ///   - interval: Binance kline interval e.g. `"15m"`
    ///   - onUpdate: called on main thread for every kline tick. First param is symbol (uppercased).
    func connect(
        symbols: [String],
        interval: String,
        onUpdate: @escaping (String, KlineData) -> Void
    ) {
        disconnect()

        guard !symbols.isEmpty else { return }

        self.symbols = symbols
        self.interval = interval
        self.onUpdate = onUpdate
        self.isShuttingDown = false
        self.reconnectCount = 0

        openSocket()
    }

    func disconnect() {
        isShuttingDown = true
        webSocket?.cancel(with: .normalClosure, reason: nil)
        webSocket = nil
        session = nil
        onUpdate = nil
    }

    // MARK: - Private

    private func openSocket() {
        let streamList = symbols
            .map { "\($0.lowercased())@kline_\(interval)" }
            .joined(separator: "/")

        guard let url = URL(string: "wss://stream.binance.com/stream?streams=\(streamList)")
        else { return }

        session = URLSession(configuration: .default, delegate: nil, delegateQueue: .main)
        webSocket = session?.webSocketTask(with: url)
        webSocket?.resume()
        receiveNext()
    }

    private func receiveNext() {
        webSocket?.receive { [weak self] result in
            guard let self, !self.isShuttingDown else { return }

            switch result {
            case .success(let message):
                switch message {
                case .string(let text):
                    self.handleMessage(text)
                case .data(let data):
                    if let text = String(data: data, encoding: .utf8) {
                        self.handleMessage(text)
                    }
                @unknown default:
                    break
                }
                self.reconnectCount = 0
                self.receiveNext()

            case .failure:
                self.scheduleReconnect()
            }
        }
    }

    private func handleMessage(_ text: String) {
        guard let data = text.data(using: .utf8),
              let message = try? JSONDecoder().decode(WSKlineMessage.self, from: data),
              let kline = message.data.k.toKlineData()
        else { return }

        let symbol = message.stream.components(separatedBy: "@").first?.uppercased() ?? ""
        onUpdate?(symbol, kline)
    }

    private func scheduleReconnect() {
        guard !isShuttingDown else { return }

        webSocket = nil
        session = nil

        let delay = min(pow(2.0, Double(reconnectCount)), 30.0)
        reconnectCount += 1

        Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard let self, !self.isShuttingDown else { return }
            self.openSocket()
        }
    }
}
