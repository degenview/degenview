import Foundation

/// The HTTP method a webhook endpoint is called with. Both carry the alert message as the body.
/// POST is TradingView's and the default.
enum WebhookHTTPMethod: String, Codable, CaseIterable, Identifiable, Sendable {
    case post = "POST"
    case put = "PUT"

    var id: String { rawValue }
}
