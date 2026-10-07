import Foundation

@testable import DegenView

/// A transport that records every request and answers from a closure, so nothing reaches the network.
final class RecordingWebhookTransport: WebhookTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [URLRequest] = []
    private let respond: @Sendable (URLRequest) async throws -> Int

    init(respond: @escaping @Sendable (URLRequest) async throws -> Int = { _ in 200 }) {
        self.respond = respond
    }

    var requests: [URLRequest] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func send(_ request: URLRequest) async throws -> Int {
        lock.lock()
        recorded.append(request)
        lock.unlock()
        return try await respond(request)
    }
}

extension URLRequest {
    var bodyString: String { String(decoding: httpBody ?? Data(), as: UTF8.self) }
    var contentType: String? { value(forHTTPHeaderField: "Content-Type") }
}

enum WebhookTestFixtures {
    static func endpoint(
        _ name: String = "Bot", url: String = "https://example.com/hook/secret-token", id: UUID = UUID(),
        enabled: Bool = true, method: WebhookHTTPMethod = .post, headers: [WebhookHeader] = []
    ) -> WebhookEndpoint {
        WebhookEndpoint(id: id, name: name, url: url, method: method, headers: headers, isEnabled: enabled)
    }
}
