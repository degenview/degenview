import Foundation

final class AlpacaWebSocketService {
    private var socket: URLSessionWebSocketTask?
    private var receiveTask: Task<Void, Never>?
    private var callback: ((String, KlineData) -> Void)?

    func connect(symbols: [String], onBar: @escaping (String, KlineData) -> Void) {
        disconnect()
        guard !symbols.isEmpty, AlpacaCredentialsStore.isConfigured else { return }
        callback = onBar
        let socket = URLSession.shared.webSocketTask(with: URL(string: "wss://stream.data.alpaca.markets/v2/iex")!)
        self.socket = socket
        socket.resume()
        receiveTask = Task { [weak self] in
            guard let self else { return }
            do {
                let credentials = AlpacaCredentialsStore.credentials
                try await send(["action": "auth", "key": credentials.keyID, "secret": credentials.secretKey])
                try await send(["action": "subscribe", "bars": symbols.map { $0.uppercased() }])
                while !Task.isCancelled { try await receive() }
            } catch { disconnect() }
        }
    }

    func disconnect() {
        receiveTask?.cancel()
        receiveTask = nil
        socket?.cancel(with: .goingAway, reason: nil)
        socket = nil
        callback = nil
    }

    private func send(_ value: [String: Any]) async throws {
        let data = try JSONSerialization.data(withJSONObject: value)
        try await socket?.send(.data(data))
    }

    private func receive() async throws {
        guard let socket else { return }
        let message = try await socket.receive()
        let data: Data
        switch message {
        case .data(let value): data = value
        case .string(let value): data = Data(value.utf8)
        @unknown default: return
        }
        guard let events = try JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return }
        for event in events where event["T"] as? String == "b" {
            guard let symbol = event["S"] as? String,
                let timestamp = event["t"] as? String,
                let date = ISO8601DateFormatter.alpaca.date(from: timestamp),
                let open = (event["o"] as? NSNumber)?.doubleValue,
                let high = (event["h"] as? NSNumber)?.doubleValue,
                let low = (event["l"] as? NSNumber)?.doubleValue,
                let close = (event["c"] as? NSNumber)?.doubleValue
            else { continue }
            let volume = (event["v"] as? NSNumber)?.doubleValue ?? 0
            let average = (event["vw"] as? NSNumber)?.doubleValue ?? 0
            let bar = KlineData(
                openTime: date, openPrice: open, highPrice: high, lowPrice: low,
                closePrice: close, volume: volume, quoteVolume: average * volume)
            Task { @MainActor [weak self] in self?.callback?(symbol, bar) }
        }
    }
}

private extension ISO8601DateFormatter {
    static let alpaca: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()
}
