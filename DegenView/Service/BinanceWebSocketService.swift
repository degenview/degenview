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
    let t: Int64  // Kline start time (ms)
    let o: String  // Open
    let h: String  // High
    let l: String  // Low
    let c: String  // Close
    let v: String  // Volume (base asset)
    let q: String  // Quote asset volume — turnover in USDT
    let x: Bool  // Is this kline closed?

    func toKlineData() -> KlineData? {
        guard let open = Double(o),
            let high = Double(h),
            let low = Double(l),
            let close = Double(c),
            let volume = Double(v)
        else { return nil }
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

private struct WSBookTickerMessage: Decodable {
    let stream: String
    let data: WSBookTickerPayload
}

private struct WSBookTickerPayload: Decodable {
    let b: String  // Best bid price
    let a: String  // Best ask price
}

// MARK: - WebSocket service

/// Manages a single Binance combined-stream WebSocket connection.
/// All callbacks fire on the main thread (URLSession delegate queue = .main).
final class BinanceWebSocketService {

    private var webSocket: URLSessionWebSocketTask?
    private var session: URLSession?
    private var onUpdate: ((String, KlineData) -> Void)?
    private var onBookTicker: ((String, Double, Double) -> Void)?
    private let bookCoalescer: BookTickerCoalescer
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
        reconnectSleep: @escaping (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) },
        bookCoalescer: BookTickerCoalescer = BookTickerCoalescer()
    ) {
        self.socketOpenObserver = socketOpenObserver
        self.reconnectSleep = reconnectSleep
        self.bookCoalescer = bookCoalescer
        bookCoalescer.deliver = { [weak self] symbol, quote in
            self?.onBookTicker?(symbol, quote.bid, quote.ask)
        }
    }

    /// Open a combined stream for the given symbols and interval.
    /// - Parameters:
    ///   - symbols: e.g. `["BTCUSDT", "ETHUSDT"]`
    ///   - interval: Binance kline interval e.g. `"15m"`
    ///   - onUpdate: called on main thread for every kline tick. First param is symbol (uppercased).
    ///   - onBookTicker: when set, also subscribes to each symbol's best bid/ask and calls this with
    ///     the symbol (uppercased), bid and ask — at most a few times a second per `BookTickerCoalescer`.
    func connect(
        symbols: [String],
        interval: String,
        onUpdate: @escaping (String, KlineData) -> Void,
        onBookTicker: ((String, Double, Double) -> Void)? = nil
    ) {
        disconnect()

        guard !symbols.isEmpty else { return }

        self.symbols = symbols
        self.interval = interval
        self.onBookTicker = onBookTicker
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
        onBookTicker = nil
        bookCoalescer.cancel()
    }

    // MARK: - Private

    private func openSocket(generation: Int) {
        guard generation == connectionGeneration, !isShuttingDown else { return }
        let streamList =
            symbols
            .flatMap { symbol -> [String] in
                let name = symbol.lowercased()
                return onBookTicker == nil ? ["\(name)@kline_\(interval)"] : ["\(name)@kline_\(interval)", "\(name)@bookTicker"]
            }
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

    /// Kept internal so frame handling can be tested without a live socket.
    func handleMessage(_ text: String) {
        guard let data = text.data(using: .utf8) else { return }
        let decoder = JSONDecoder()

        if let message = try? decoder.decode(WSKlineMessage.self, from: data) {
            guard let kline = message.data.k.toKlineData() else { return }
            let symbol = message.stream.components(separatedBy: "@").first?.uppercased() ?? ""
            onUpdate?(symbol, kline)
        } else if onBookTicker != nil, let message = try? decoder.decode(WSBookTickerMessage.self, from: data),
            let bid = Double(message.data.b), let ask = Double(message.data.a)
        {
            let symbol = message.stream.components(separatedBy: "@").first?.uppercased() ?? ""
            bookCoalescer.submit(symbol: symbol, bid: bid, ask: ask)
        }
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
