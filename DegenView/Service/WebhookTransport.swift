import Foundation

/// The network seam of webhook delivery: sends one request and reports the HTTP status.
/// Tests substitute a recording fake, so nothing in the suite touches the Internet.
protocol WebhookTransport: Sendable {
    /// Returns the HTTP status code. Throws `URLError` (or any error) when no response arrived.
    func send(_ request: URLRequest) async throws -> Int
}

/// `URLSession` transport. Uses its own ephemeral session: no cookies, no cache, no shared
/// credentials, and redirects are refused. A 3xx therefore surfaces as a failed delivery, and a
/// redirect cannot move a validated URL to somewhere the policy never saw.
struct URLSessionWebhookTransport: WebhookTransport {
    private let session: URLSession

    init() {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = WebhookDeliveryService.requestTimeout
        configuration.timeoutIntervalForResource = WebhookDeliveryService.requestTimeout
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        session = URLSession(configuration: configuration, delegate: RedirectRefuser(), delegateQueue: nil)
    }

    func send(_ request: URLRequest) async throws -> Int {
        let (_, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
        return http.statusCode
    }

    private final class RedirectRefuser: NSObject, URLSessionTaskDelegate {
        func urlSession(
            _ session: URLSession, task: URLSessionTask,
            willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
            completionHandler: @escaping (URLRequest?) -> Void
        ) {
            completionHandler(nil)
        }
    }
}
