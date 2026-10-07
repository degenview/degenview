import Foundation

enum WebhookEndpointError: LocalizedError, Equatable {
    case invalidURL(WebhookDeliveryError)
    case invalidHeaders
    case invalidSecret(WebhookSecretError)
    /// The URL or a header uses `{{secret}}` but no secret is saved.
    case secretRequired
    case emptyName
    case notFound
    /// The saved endpoints could not be read, so nothing may be written over them.
    case persistenceUnavailable
    case saveFailed
    /// The Keychain refused the secret.
    case secretSaveFailed

    var errorDescription: String? {
        switch self {
        case .invalidURL(let reason): WebhookURLPolicy.message(for: reason)
        case .invalidHeaders: "One or more headers need fixing."
        case .invalidSecret(let reason): WebhookHeaderPolicy.message(for: reason)
        case .secretRequired: "Add the secret that {{secret}} stands for."
        case .emptyName: "Enter a name for this webhook."
        case .notFound: "This webhook no longer exists."
        case .persistenceUnavailable: "Saved webhooks could not be read, so changes are turned off."
        case .saveFailed: "The webhook could not be saved."
        case .secretSaveFailed: "The secret could not be saved to your Keychain."
        }
    }
}

/// The user's webhook endpoints. Names, URL and header templates live in the `webhook_endpoint`
/// table; each endpoint's secret lives only in the Keychain (`WebhookSecretStore`), referenced from
/// the templates as `{{secret}}`. This is the only writer; the login-item agent reads the same rows
/// and Keychain items when it delivers a price alert.
@MainActor
final class WebhookEndpointStore: ObservableObject {
    static let shared = WebhookEndpointStore()

    @Published private(set) var endpoints: [WebhookEndpoint] = []
    /// True when the saved endpoints could not be read. An empty list must not be written back over
    /// data that failed to load, so every change is refused until the next launch.
    @Published private(set) var loadFailed = false
    /// Endpoints with a saved secret, checked once and after each change, so a view refresh never
    /// reaches the Keychain.
    @Published private(set) var secretIDs: Set<UUID> = []

    private let database: AppDatabase
    private let secrets: any WebhookSecretStore
    private let references: any WebhookReferenceSource

    init(
        database: AppDatabase = .shared,
        secrets: any WebhookSecretStore = KeychainWebhookSecretStore.shared,
        references: (any WebhookReferenceSource)? = nil
    ) {
        self.database = database
        self.secrets = secrets
        self.references = references ?? AlertWebhookReferences()
        do {
            endpoints = try database.webhookEndpoints()
            secretIDs = Set(endpoints.map(\.id).filter { secrets.hasSecret(for: $0) })
        } catch {
            loadFailed = true
            #if DEBUG
                print("[WebhookEndpointStore] Could not read endpoints: \(error.localizedDescription)")
            #endif
        }
    }

    func endpoint(id: UUID) -> WebhookEndpoint? { endpoints.first { $0.id == id } }

    func hasSecret(_ id: UUID) -> Bool { secretIDs.contains(id) }

    /// For the editor only: the saved value, to show masked and let the user change it.
    func secret(for id: UUID) -> String? { secrets.secret(for: id) }

    /// `https://example.com/•••`: what a row may show of the URL.
    func address(for id: UUID) -> String {
        guard let url = endpoint(id: id)?.url, !url.isEmpty else { return "No address" }
        return WebhookRedactor.redact(url)
    }

    func referenceCount(to id: UUID) -> Int { references.referenceCount(to: id) }

    /// - Parameter secret: the value `{{secret}}` stands for; empty means none.
    @discardableResult
    func add(
        name: String, description: String, url: String, method: WebhookHTTPMethod = .post,
        headers: [WebhookHeader] = [], secret: String = "", isEnabled: Bool
    ) throws -> WebhookEndpoint {
        guard !loadFailed else { throw WebhookEndpointError.persistenceUnavailable }
        let headers = headers.filter { !$0.isBlank }
        let cleanName = try validate(
            name: name, url: url, headers: headers, secret: secret, hasStoredSecret: false)
        let endpoint = WebhookEndpoint(
            name: cleanName, url: url.trimmingCharacters(in: .whitespacesAndNewlines), method: method,
            headers: Self.cleaned(headers),
            description: description.trimmingCharacters(in: .whitespacesAndNewlines), isEnabled: isEnabled)
        try persist(endpoints + [endpoint], secret: secret.isEmpty ? .keep : .set(secret), for: endpoint.id)
        return endpoint
    }

    /// Renaming or editing keeps the id, so alerts keep pointing at this endpoint. A nil `url`,
    /// `method`, `headers` or `secret` keeps what is saved; an empty `secret` removes it.
    func update(
        id: UUID, name: String, description: String, url: String?, method: WebhookHTTPMethod? = nil,
        headers: [WebhookHeader]? = nil, secret: String? = nil, isEnabled: Bool
    ) throws {
        guard !loadFailed else { throw WebhookEndpointError.persistenceUnavailable }
        guard var endpoint = endpoint(id: id) else { throw WebhookEndpointError.notFound }
        let newURL = (url ?? endpoint.url).trimmingCharacters(in: .whitespacesAndNewlines)
        let newHeaders = (headers ?? endpoint.headers).filter { !$0.isBlank }
        let keepsSecret = secret == nil && secrets.hasSecret(for: id)
        let cleanName = try validate(
            name: name, url: newURL, headers: newHeaders, secret: secret ?? "", hasStoredSecret: keepsSecret)
        endpoint.name = cleanName
        endpoint.description = description.trimmingCharacters(in: .whitespacesAndNewlines)
        endpoint.url = newURL
        endpoint.headers = Self.cleaned(newHeaders)
        if let method { endpoint.method = method }
        endpoint.isEnabled = isEnabled
        endpoint.updatedAt = Date()
        let change: SecretChange = secret.map { $0.isEmpty ? .remove : .set($0) } ?? .keep
        try persist(endpoints.map { $0.id == id ? endpoint : $0 }, secret: change, for: id)
    }

    func setEnabled(_ enabled: Bool, for id: UUID) throws {
        guard !loadFailed else { throw WebhookEndpointError.persistenceUnavailable }
        guard var endpoint = endpoint(id: id) else { throw WebhookEndpointError.notFound }
        guard endpoint.isEnabled != enabled else { return }
        endpoint.isEnabled = enabled
        endpoint.updatedAt = Date()
        try persist(endpoints.map { $0.id == id ? endpoint : $0 }, secret: .keep, for: id)
    }

    /// Removes the endpoint, its Keychain secret and its id from every alert that used it. The row
    /// goes first: a leftover Keychain item is harmless, a leftover row without its secret is not.
    func delete(id: UUID) throws {
        guard !loadFailed else { throw WebhookEndpointError.persistenceUnavailable }
        guard endpoint(id: id) != nil else { throw WebhookEndpointError.notFound }
        try persist(endpoints.filter { $0.id != id }, secret: .keep, for: id)
        secrets.remove(for: id)
        secretIDs.remove(id)
        references.removeReferences(to: id)
    }

    // MARK: - Private

    private enum SecretChange {
        case keep
        case set(String)
        case remove
    }

    private func validate(
        name: String, url: String, headers: [WebhookHeader], secret: String, hasStoredSecret: Bool
    ) throws -> String {
        let cleanName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanName.isEmpty else { throw WebhookEndpointError.emptyName }
        if headers.count > WebhookHeaderPolicy.maximumHeaders { throw WebhookEndpointError.invalidHeaders }
        if let problem = WebhookRequestResolver.validateTemplates(url: url, headers: headers) {
            throw problem == .invalidHeader ? WebhookEndpointError.invalidHeaders : .invalidURL(problem)
        }
        if !secret.isEmpty, let problem = WebhookHeaderPolicy.validate(secret: secret) {
            throw WebhookEndpointError.invalidSecret(problem)
        }
        if WebhookRequestResolver.references(url: url, headers: headers), secret.isEmpty, !hasStoredSecret {
            throw WebhookEndpointError.secretRequired
        }
        return cleanName
    }

    /// Header names and values are saved trimmed.
    private static func cleaned(_ headers: [WebhookHeader]) -> [WebhookHeader] {
        headers.map {
            WebhookHeader(
                id: $0.id, name: $0.name.trimmingCharacters(in: .whitespaces),
                value: $0.value.trimmingCharacters(in: .whitespaces))
        }
    }

    /// Writes the Keychain first, then the table. If the table write fails the Keychain goes back to
    /// what it held, so the two never disagree about whether a secret exists.
    private func persist(_ updated: [WebhookEndpoint], secret: SecretChange, for id: UUID) throws {
        var previous: String?
        if case .keep = secret {} else { previous = secrets.secret(for: id) }
        switch secret {
        case .keep: break
        case .set(let value):
            do { try secrets.setSecret(value, for: id) } catch { throw WebhookEndpointError.secretSaveFailed }
        case .remove:
            secrets.remove(for: id)
        }
        do {
            try database.replaceWebhookEndpoints(updated)
        } catch {
            switch secret {
            case .keep: break
            case .set, .remove:
                if let previous {
                    try? secrets.setSecret(previous, for: id)
                } else {
                    secrets.remove(for: id)
                }
            }
            throw WebhookEndpointError.saveFailed
        }
        endpoints = updated
        if updated.contains(where: { $0.id == id }) {
            if secrets.hasSecret(for: id) { secretIDs.insert(id) } else { secretIDs.remove(id) }
        }
    }
}
