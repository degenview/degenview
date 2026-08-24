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
    let v: String  // Volume (base asset)
    let q: String  // Quote asset volume — turnover in USDT
    let x: Bool    // Is this kline closed?

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
            volume: volume,
            quoteVolume: Double(q) ?? 0,
            isClosed: x
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
    private var reconnectTask: Task<Void, Never>?
    private var connectionGeneration = 0
    private let socketOpenObserver: ((URL) -> Void)?
    private let reconnectSleep: (UInt64) async throws -> Void

    init(
        socketOpenObserver: ((URL) -> Void)? = nil,
        reconnectSleep: @escaping (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) }
    ) {
        self.socketOpenObserver = socketOpenObserver
        self.reconnectSleep = reconnectSleep
    }

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
        self.connectionGeneration += 1

        openSocket(generation: connectionGeneration)
    }

    func disconnect() {
        connectionGeneration += 1
        isShuttingDown = true
        reconnectTask?.cancel()
        reconnectTask = nil
        webSocket?.cancel(with: .normalClosure, reason: nil)
        webSocket = nil
        session = nil
        onUpdate = nil
    }

    // MARK: - Private

    private func openSocket(generation: Int) {
        guard generation == connectionGeneration, !isShuttingDown else { return }
        let streamList = symbols
            .map { "\($0.lowercased())@kline_\(interval)" }
            .joined(separator: "/")

        guard let url = URL(string: "wss://stream.binance.com/stream?streams=\(streamList)")
        else { return }

        if let socketOpenObserver {
            socketOpenObserver(url)
            return
        }

        session = URLSession(configuration: .default, delegate: nil, delegateQueue: .main)
        webSocket = session?.webSocketTask(with: url)
        webSocket?.resume()
        receiveNext(generation: generation)
    }

    private func receiveNext(generation: Int) {
        webSocket?.receive { [weak self] result in
            guard let self,
                  !self.isShuttingDown,
                  generation == self.connectionGeneration
            else { return }

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
                self.receiveNext(generation: generation)

            case .failure:
                self.connectionDidFail(generation: generation)
            }
        }
    }

    /// Kept internal so connection lifecycle can be tested without a live socket.
    func connectionDidFail(generation: Int) {
        scheduleReconnect(generation: generation)
    }

    var currentConnectionGeneration: Int { connectionGeneration }

    private func handleMessage(_ text: String) {
        guard let data = text.data(using: .utf8),
              let message = try? JSONDecoder().decode(WSKlineMessage.self, from: data),
              let kline = message.data.k.toKlineData()
        else { return }

        let symbol = message.stream.components(separatedBy: "@").first?.uppercased() ?? ""
        onUpdate?(symbol, kline)
    }

    private func scheduleReconnect(generation: Int) {
        guard !isShuttingDown, generation == connectionGeneration else { return }

        reconnectTask?.cancel()
        webSocket = nil
        session = nil

        let delay = min(pow(2.0, Double(reconnectCount)), 30.0)
        reconnectCount += 1

        reconnectTask = Task { [weak self] in
            do {
                try await self?.reconnectSleep(UInt64(delay * 1_000_000_000))
            } catch {
                return
            }
            guard let self,
                  !self.isShuttingDown,
                  generation == self.connectionGeneration
            else { return }
            self.reconnectTask = nil
            self.openSocket(generation: generation)
        }
    }
}
