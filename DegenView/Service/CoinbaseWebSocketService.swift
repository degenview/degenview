import Foundation

/// One Coinbase Exchange market-data socket for a set of products.
///
/// Mirrors `BinanceWebSocketService` — same lifecycle, generation guard and backoff — but
/// Coinbase has no kline stream. The `ticker` channel reports each trade, and the chart
/// folds those into its live candle. Because nothing here depends on the chart interval,
/// a timeframe change doesn't need a reconnect.
///
/// All callbacks fire on the main thread (URLSession delegate queue = .main).
final class CoinbaseWebSocketService {
    static let feedURL = URL(string: "wss://ws-feed.exchange.coinbase.com")!

    private var webSocket: URLSessionWebSocketTask?
    private var session: URLSession?
    private var onTick: ((CoinbaseTick) -> Void)?
    private var products: [String] = []
    private var reconnectCount = 0
    private var isShuttingDown = false
    private var reconnectTask: Task<Void, Never>?
    private var watchdogTask: Task<Void, Never>?
    private var connectionGeneration = 0
    private var lastMessageAt = Date()
    /// Highest trade seen per product. A reconnect replays the latest trade; it must not count twice.
    private var lastTradeIDs: [String: Int64] = [:]
    private let socketOpenObserver: ((URL) -> Void)?
    private let reconnectSleep: (UInt64) async throws -> Void

    /// How long the feed may stay silent before the socket is treated as dead. The
    /// `heartbeat` channel sends a message per product every second, so silence means a stall.
    private let silenceLimit: TimeInterval = 20

    init(
        socketOpenObserver: ((URL) -> Void)? = nil,
        reconnectSleep: @escaping (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) }
    ) {
        self.socketOpenObserver = socketOpenObserver
        self.reconnectSleep = reconnectSleep
    }

    /// Open a socket for the given products.
    /// - Parameters:
    ///   - products: Coinbase ids, e.g. `["BTC-USD", "ETH-USD"]`
    ///   - onTick: called on the main thread for every trade.
    func connect(products: [String], onTick: @escaping (CoinbaseTick) -> Void) {
        disconnect()

        guard !products.isEmpty else { return }

        var seen = Set<String>()
        self.products = products.map { $0.uppercased() }.filter { seen.insert($0).inserted }
        self.onTick = onTick
        self.isShuttingDown = false
        self.reconnectCount = 0
        self.lastTradeIDs = [:]
        self.connectionGeneration += 1

        openSocket(generation: connectionGeneration)
    }

    func disconnect() {
        connectionGeneration += 1
        isShuttingDown = true
        reconnectTask?.cancel()
        reconnectTask = nil
        watchdogTask?.cancel()
        watchdogTask = nil
        webSocket?.cancel(with: .normalClosure, reason: nil)
        webSocket = nil
        session = nil
        onTick = nil
    }

    /// The frame that starts the feed. Coinbase drops a socket that hasn't subscribed within 5 seconds.
    static func subscribeMessage(products: [String]) -> String {
        let payload: [String: Any] = [
            "type": "subscribe",
            "product_ids": products,
            "channels": ["ticker", "heartbeat"],
        ]
        let data = (try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }

    // MARK: - Private

    private func openSocket(generation: Int) {
        guard generation == connectionGeneration, !isShuttingDown else { return }

        if let socketOpenObserver {
            socketOpenObserver(Self.feedURL)
            return
        }

        session = URLSession(configuration: .default, delegate: nil, delegateQueue: .main)
        webSocket = session?.webSocketTask(with: Self.feedURL)
        webSocket?.resume()
        lastMessageAt = Date()
        webSocket?.send(.string(Self.subscribeMessage(products: products))) { [weak self] error in
            guard error != nil, let self else { return }
            self.connectionDidFail(generation: generation)
        }
        receiveNext(generation: generation)
        startWatchdog(generation: generation)
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
                self.lastMessageAt = Date()
                self.reconnectCount = 0
                self.receiveNext(generation: generation)

            case .failure:
                self.connectionDidFail(generation: generation)
            }
        }
    }

    /// A socket can go quiet without ever reporting an error. Heartbeats make silence detectable.
    private func startWatchdog(generation: Int) {
        watchdogTask?.cancel()
        watchdogTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                guard let self, !Task.isCancelled else { return }
                guard generation == self.connectionGeneration, !self.isShuttingDown else { return }
                if Date().timeIntervalSince(self.lastMessageAt) > self.silenceLimit {
                    self.connectionDidFail(generation: generation)
                    return
                }
            }
        }
    }

    /// Kept internal so connection lifecycle can be tested without a live socket.
    func connectionDidFail(generation: Int) {
        scheduleReconnect(generation: generation)
    }

    var currentConnectionGeneration: Int { connectionGeneration }

    /// Internal for tests. Only `ticker` frames produce a callback.
    func handleMessage(_ text: String) {
        guard let data = text.data(using: .utf8),
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let tick = CoinbaseTick(json: json)
        else { return }

        if let seen = lastTradeIDs[tick.productID], tick.tradeID <= seen { return }
        lastTradeIDs[tick.productID] = tick.tradeID
        onTick?(tick)
    }

    private func scheduleReconnect(generation: Int) {
        guard !isShuttingDown, generation == connectionGeneration else { return }

        reconnectTask?.cancel()
        watchdogTask?.cancel()
        watchdogTask = nil
        webSocket?.cancel(with: .goingAway, reason: nil)
        webSocket = nil
        session = nil

        let delay = min(pow(2.0, Double(reconnectCount)), 30.0)
        reconnectCount += 1

        reconnectTask = Task { @MainActor [weak self] in
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
