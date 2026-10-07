import Foundation

/// The HTTP body and Content-Type for a rendered alert message.
///
/// The message is the body, byte for byte: nothing is wrapped, reformatted or terminated with a
/// newline. Only the Content-Type depends on the content, as on TradingView: JSON when the whole
/// message parses as a JSON object or array, plain text otherwise.
struct WebhookPayload: Equatable, Sendable {
    static let jsonContentType = "application/json; charset=utf-8"
    static let textContentType = "text/plain; charset=utf-8"

    let body: Data
    let contentType: String

    init(message: String) {
        body = Data(message.utf8)
        contentType = Self.isJSON(body) ? Self.jsonContentType : Self.textContentType
    }

    /// Real parsing, not a prefix check. Bare scalars such as `123` or `true` are not treated as
    /// JSON: nobody configures a webhook to receive a lone number, and a plain word like `BUY`
    /// must stay text either way.
    static func isJSON(_ data: Data) -> Bool {
        guard let object = try? JSONSerialization.jsonObject(with: data, options: []) else { return false }
        return object is [String: Any] || object is [Any]
    }
}
