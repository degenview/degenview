import Foundation

/// Sends rendered alert messages to webhook endpoints. The only code that performs webhook HTTP:
/// price alerts (in the app or the login-item agent), Pine alerts and the Settings test button all
/// come through here, so validation, Content-Type, timeout and result classification are one path.
///
/// One call is one attempt. There is deliberately no retry: an endpoint is often a trading bot,
/// and a retried "BUY" can become a second order. The result carries everything a retry policy
/// would need, so one could be added explicitly later.
actor WebhookDeliveryService {
    /// TradingView cancels a request that takes longer than three seconds.
    static let requestTimeout: TimeInterval = 3

    enum Mode: Sendable {
        /// Alert delivery: a disabled endpoint is skipped.
        case production
        /// The Settings Test button: the user may be testing before enabling, so the enabled flag is
        /// bypassed. The URL is still validated like any other.
        case test
    }

    static let shared = WebhookDeliveryService()

    private let transport: any WebhookTransport
    private let secrets: any WebhookSecretStore
    private let now: @Sendable () -> Date

    init(
        transport: any WebhookTransport = URLSessionWebhookTransport(),
        secrets: any WebhookSecretStore = KeychainWebhookSecretStore.shared,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.transport = transport
        self.secrets = secrets
        self.now = now
    }

    /// One attempt at one endpoint. Never throws: every outcome is in the result.
    func deliver(
        message: String, to endpoint: WebhookEndpoint, mode: Mode = .production
    ) async -> WebhookDeliveryResult {
        let clock = ContinuousClock()
        let started = clock.now
        let timestamp = now()

        func result(_ error: WebhookDeliveryError?, status: Int? = nil) -> WebhookDeliveryResult {
            let elapsed = started.duration(to: clock.now)
            let seconds = Double(elapsed.components.seconds) + Double(elapsed.components.attoseconds) / 1e18
            return WebhookDeliveryResult(
                endpointID: endpoint.id, timestamp: timestamp, duration: seconds, statusCode: status, error: error)
        }

        if case .production = mode, !endpoint.isEnabled { return result(.disabledEndpoint) }
        // The Keychain is read per attempt, and only when something uses the secret, so an edit
        // applies at once and an endpoint without one never touches it.
        let secret = endpoint.referencesSecret ? secrets.secret(for: endpoint.id) : nil
        let resolved: WebhookResolvedRequest
        switch WebhookRequestResolver.resolve(url: endpoint.url, headers: endpoint.headers, secret: secret) {
        case .success(let value): resolved = value
        case .failure(let error): return result(error)
        }

        var request = URLRequest(
            url: resolved.url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: Self.requestTimeout)
        request.httpMethod = endpoint.method.rawValue
        for header in resolved.headers { request.setValue(header.value, forHTTPHeaderField: header.name) }
        // Set last: the body's Content-Type is DegenView's to decide, and the secret is never in the body.
        let payload = WebhookPayload(message: message)
        request.httpBody = payload.body
        request.setValue(payload.contentType, forHTTPHeaderField: "Content-Type")

        let outcome: WebhookDeliveryResult
        do {
            let status = try await transport.send(request)
            outcome = result((200...299).contains(status) ? nil : .httpFailure, status: status)
        } catch let error as URLError where error.code == .timedOut {
            outcome = result(.timeout)
        } catch {
            outcome = result(.networkFailure)
        }
        #if DEBUG
            if !outcome.succeeded { print("[Webhook] delivery failed \(WebhookRedactor.logLine(outcome))") }
        #endif
        return outcome
    }

    /// Delivers to every endpoint independently and concurrently: one failing or slow endpoint
    /// never holds up or suppresses another. Disabled endpoints are left out of the results, since
    /// skipping one is a switch the user set, not a failure.
    func deliverAll(
        message: String, to endpoints: [WebhookEndpoint], mode: Mode = .production
    ) async -> [WebhookDeliveryResult] {
        await withTaskGroup(of: WebhookDeliveryResult.self) { group in
            for endpoint in endpoints {
                group.addTask { await self.deliver(message: message, to: endpoint, mode: mode) }
            }
            var results: [WebhookDeliveryResult] = []
            for await result in group where result.error != .disabledEndpoint { results.append(result) }
            return results
        }
    }
}
