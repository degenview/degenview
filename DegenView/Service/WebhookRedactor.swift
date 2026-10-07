import Foundation

/// Webhook URLs are credentials. Anything shown in a row, a log line or a description goes
/// through here first.
enum WebhookRedactor {
    /// `https://example.com/•••`: the scheme and host stay so the user can tell endpoints apart;
    /// path, query, fragment and userinfo never appear. Unparseable input yields a bare marker.
    static func redact(_ urlString: String) -> String {
        guard
            let components = URLComponents(
                string: urlString.trimmingCharacters(in: .whitespacesAndNewlines)),
            let scheme = components.scheme, let host = components.host, !host.isEmpty
        else { return "•••" }
        let port = components.port.map { ":\($0)" } ?? ""
        let hasMore = !(components.path.isEmpty || components.path == "/") || components.query != nil
        return "\(scheme)://\(host)\(port)/" + (hasMore ? "•••" : "")
    }

    static func redact(_ url: URL) -> String { redact(url.absoluteString) }

    /// The log line for a finished attempt: ids, status and timing only.
    static func logLine(_ result: WebhookDeliveryResult) -> String {
        var parts = ["endpointID=\(result.endpointID.uuidString)"]
        if let status = result.statusCode { parts.append("status=\(status)") }
        if let error = result.error { parts.append("error=\(error.rawValue)") }
        parts.append(String(format: "duration=%.2f", result.duration))
        return parts.joined(separator: " ")
    }
}
