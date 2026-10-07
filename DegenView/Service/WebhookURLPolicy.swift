import Foundation

/// The one place that decides which webhook URLs are acceptable.
///
/// Policy: `http` and `https` only, a host is required, any port. Loopback, link-local and
/// private-network hosts are allowed on purpose: DegenView is a desktop app and the endpoints are
/// configured by its owner, and local trading bridges and dev servers are a legitimate target.
/// This is looser than TradingView, which only reaches ports 80 and 443.
enum WebhookURLPolicy {
    static let allowedSchemes: Set<String> = ["http", "https"]

    /// Parses and validates `string`. The error is typed so callers pick their own wording.
    static func validate(_ string: String) -> Result<URL, WebhookDeliveryError> {
        let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let components = URLComponents(string: trimmed) else {
            return .failure(.invalidURL)
        }
        guard let scheme = components.scheme?.lowercased() else { return .failure(.invalidURL) }
        guard allowedSchemes.contains(scheme) else { return .failure(.unsupportedScheme) }
        guard let host = components.host, !host.isEmpty else { return .failure(.invalidURL) }
        if let port = components.port, !(1...65535).contains(port) {
            return .failure(.unsupportedPort)
        }
        guard let url = components.url, url.host != nil else { return .failure(.invalidURL) }
        return .success(url)
    }

    /// Whether a redirect target would pass validation. Redirects are not followed today, so this
    /// only backs the transport's refusal; it keeps the rule next to the policy it mirrors.
    static func isAcceptable(_ url: URL) -> Bool {
        if case .success = validate(url.absoluteString) { return true }
        return false
    }

    /// A message for the editor. Never includes the URL.
    static func message(for error: WebhookDeliveryError) -> String {
        switch error {
        case .unsupportedScheme: "Use an http:// or https:// address."
        case .unsupportedPort: "The port must be between 1 and 65535."
        case .secretInHost: "The secret can't go in the host. Use it in the path or after a ?."
        default: "Enter a full address, such as https://example.com/hook."
        }
    }
}
