import Foundation

/// A reusable webhook destination, configured once in Settings and referenced by id from
/// price alerts and Pine alert subscriptions.
///
/// The URL and header values are templates stored as plain text in the `webhook_endpoint` row, so the
/// login-item agent can read them. A secret goes in as `{{secret}}`; its value lives only in the
/// Keychain (`WebhookSecretStore`). UI and logs only ever show `WebhookRedactor.redact(url)`. Renaming an endpoint keeps its id, so everything that points at it
/// keeps working.
struct WebhookEndpoint: Identifiable, Codable, Equatable, Sendable {
    let id: UUID
    var name: String
    var description: String
    var url: String
    var method: WebhookHTTPMethod
    /// Custom headers sent with every request. Values may use `{{secret}}`.
    var headers: [WebhookHeader]
    var isEnabled: Bool
    var createdAt: Date
    var updatedAt: Date

    init(
        id: UUID = UUID(),
        name: String,
        url: String,
        method: WebhookHTTPMethod = .post,
        headers: [WebhookHeader] = [],
        description: String = "",
        isEnabled: Bool = true,
        createdAt: Date = Date(),
        updatedAt: Date? = nil
    ) {
        self.id = id
        self.name = name
        self.url = url
        self.method = method
        self.headers = headers
        self.description = description
        self.isEnabled = isEnabled
        self.createdAt = createdAt
        self.updatedAt = updatedAt ?? createdAt
    }
}

extension WebhookEndpoint {
    /// Whether the URL or a header asks for the Keychain secret.
    var referencesSecret: Bool { WebhookRequestResolver.references(url: url, headers: headers) }

    private enum CodingKeys: String, CodingKey {
        case id, name, description, url, method, headers, isEnabled, createdAt, updatedAt
    }

    /// A row without a `url` or `method` (older data) still loads, with an empty one that fails validation when used, instead
    /// of making the whole endpoint list unreadable.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UUID.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        description = try c.decode(String.self, forKey: .description)
        url = try c.decodeIfPresent(String.self, forKey: .url) ?? ""
        // A missing or unrecognised method falls back to POST rather than dropping the endpoint.
        method = (try? c.decodeIfPresent(WebhookHTTPMethod.self, forKey: .method)).flatMap { $0 } ?? .post
        headers = try c.decodeIfPresent([WebhookHeader].self, forKey: .headers) ?? []
        isEnabled = try c.decode(Bool.self, forKey: .isEnabled)
        createdAt = try c.decode(Date.self, forKey: .createdAt)
        updatedAt = try c.decode(Date.self, forKey: .updatedAt)
    }
}
