import Foundation

/// A request ready to send: the final URL and headers, with the secret already in.
struct WebhookResolvedRequest: Equatable, Sendable {
    var url: URL
    var headers: [WebhookHeader]
}

/// Turns an endpoint's templates and its Keychain secret into a concrete request.
///
/// The secret is encoded for where it lands. In the URL it is percent-encoded with the RFC 3986
/// unreserved set only (`A-Z a-z 0-9 - . _ ~`), so one rule is right in a path segment, a query
/// value or a fragment: `/`, `?`, `&`, `=`, `#`, `+`, `%`, spaces and non-ASCII are all escaped.
/// In a header value it goes in as is. It is never allowed in the scheme, user info, host or port.
/// The result holds the secret, so it must never be logged or stored.
enum WebhookRequestResolver {
    private static let unreserved = CharacterSet(
        charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")

    /// Percent-encodes `secret` for use inside a URL.
    static func urlEncoded(_ secret: String) -> String {
        secret.addingPercentEncoding(withAllowedCharacters: unreserved) ?? ""
    }

    /// Checks everything that does not need the secret value: the URL's shape and where the secret
    /// sits in it, and the headers. The editor and the store use this before saving.
    static func validateTemplates(url template: String, headers: [WebhookHeader]) -> WebhookDeliveryError? {
        let trimmed = template.trimmingCharacters(in: .whitespacesAndNewlines)
        if secretIsInAuthority(trimmed) { return .secretInHost }
        if case .failure(let error) = WebhookURLPolicy.validate(
            WebhookSecretPlaceholder.replacing(in: trimmed, with: "x"))
        {
            return error
        }
        guard WebhookHeaderPolicy.isValid(headers) else { return .invalidHeader }
        return nil
    }

    static func references(url template: String, headers: [WebhookHeader]) -> Bool {
        WebhookSecretPlaceholder.contains(template) || headers.contains { WebhookSecretPlaceholder.contains($0.value) }
    }

    /// - Parameter secret: required only when the URL or a header uses `{{secret}}`; an unused
    ///   secret is ignored. Nil means it could not be read.
    static func resolve(
        url template: String, headers: [WebhookHeader], secret: String?
    ) -> Result<WebhookResolvedRequest, WebhookDeliveryError> {
        if let problem = validateTemplates(url: template, headers: headers) { return .failure(problem) }
        let trimmed = template.trimmingCharacters(in: .whitespacesAndNewlines)

        let usesSecret = references(url: trimmed, headers: headers)
        if usesSecret && (secret?.isEmpty ?? true) { return .failure(.secretUnavailable) }

        let url: URL
        switch WebhookURLPolicy.validate(
            WebhookSecretPlaceholder.replacing(in: trimmed, with: urlEncoded(secret ?? "")))
        {
        case .success(let valid): url = valid
        case .failure(let error): return .failure(error)
        }

        var resolved: [WebhookHeader] = []
        for header in headers {
            let value = WebhookSecretPlaceholder.replacing(
                in: header.value.trimmingCharacters(in: .whitespaces), with: secret ?? "")
            // The secret may add characters the template did not have.
            guard WebhookHeaderPolicy.validate(value: value) == nil else { return .failure(.invalidHeader) }
            resolved.append(
                WebhookHeader(id: header.id, name: header.name.trimmingCharacters(in: .whitespaces), value: value))
        }
        return .success(WebhookResolvedRequest(url: url, headers: resolved))
    }

    /// The URL as the editor previews it: the secret shown as bullets, nothing sensitive.
    static func maskedURL(_ template: String) -> String {
        WebhookSecretPlaceholder.replacing(
            in: template.trimmingCharacters(in: .whitespacesAndNewlines), with: "••••••")
    }

    /// Whether the placeholder sits in the scheme, user info, host or port, i.e. before the path.
    static func secretIsInAuthority(_ template: String) -> Bool {
        guard WebhookSecretPlaceholder.contains(template) else { return false }
        // Without a scheme the URL policy reports the real problem.
        guard let schemeEnd = template.range(of: "://") else { return false }
        let rest = template[schemeEnd.upperBound...]
        let authorityEnd = rest.firstIndex { "/?#".contains($0) } ?? rest.endIndex
        return WebhookSecretPlaceholder.contains(String(template[..<schemeEnd.lowerBound]))
            || WebhookSecretPlaceholder.contains(String(rest[..<authorityEnd]))
    }
}
