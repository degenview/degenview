import Foundation

/// The "Use secret…" shortcuts: each edits the URL or headers so a non-technical user never has to
/// type `{{secret}}`. All pure and idempotent, so pressing one twice changes nothing the second time.
enum WebhookSecretPlacement {
    enum Choice: Equatable {
        /// Add `?name={{secret}}` (or `&name=…`) to the URL.
        case urlParameter(String)
        /// `Authorization: Bearer {{secret}}`
        case bearerToken
        /// `X-API-Key: {{secret}}`
        case apiKey
        /// A new header whose name the user fills in.
        case customHeader
    }

    struct Edit: Equatable {
        var url: String
        var headers: [WebhookHeader]
        /// Whether the change lands in the headers section, so the editor can open it.
        var touchesHeaders: Bool
    }

    static func apply(_ choice: Choice, url: String, headers: [WebhookHeader]) -> Edit {
        switch choice {
        case .urlParameter(let name):
            return Edit(url: addingParameter(name, to: url), headers: headers, touchesHeaders: false)
        case .bearerToken:
            return Edit(
                url: url,
                headers: setting("Authorization", to: "Bearer \(WebhookSecretPlaceholder.token)", in: headers),
                touchesHeaders: true)
        case .apiKey:
            return Edit(
                url: url, headers: setting("X-API-Key", to: WebhookSecretPlaceholder.token, in: headers),
                touchesHeaders: true)
        case .customHeader:
            let alreadyThere = headers.contains {
                $0.name.trimmingCharacters(in: .whitespaces).isEmpty && WebhookSecretPlaceholder.contains($0.value)
            }
            let added = alreadyThere ? headers : headers + [WebhookHeader(value: WebhookSecretPlaceholder.token)]
            return Edit(url: url, headers: added, touchesHeaders: true)
        }
    }

    /// Appends the parameter before any `#fragment`, with `?` or `&` as the URL needs. A URL that
    /// already uses the secret, or is empty, is returned unchanged.
    static func addingParameter(_ name: String, to url: String) -> String {
        let trimmed = url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !WebhookSecretPlaceholder.contains(trimmed) else { return url }
        let fragmentStart = trimmed.firstIndex(of: "#")
        let base = fragmentStart.map { String(trimmed[..<$0]) } ?? trimmed
        let fragment = fragmentStart.map { String(trimmed[$0...]) } ?? ""
        let separator: String
        if base.contains("?") {
            separator = base.hasSuffix("?") || base.hasSuffix("&") ? "" : "&"
        } else {
            separator = "?"
        }
        return base + separator + name + "=" + WebhookSecretPlaceholder.token + fragment
    }

    /// Sets `name` (case-insensitively) if a header of that name exists, else appends it. A header
    /// that already carries the secret is left as is.
    private static func setting(_ name: String, to value: String, in headers: [WebhookHeader]) -> [WebhookHeader] {
        var result = headers
        if let index = result.firstIndex(where: {
            $0.name.trimmingCharacters(in: .whitespaces).caseInsensitiveCompare(name) == .orderedSame
        }) {
            if !WebhookSecretPlaceholder.contains(result[index].value) { result[index].value = value }
        } else {
            result.append(WebhookHeader(name: name, value: value))
        }
        return result
    }

    // MARK: Credential nudge

    /// A header that looks like it carries a credential typed in plain, which would otherwise be saved
    /// in the database instead of the Keychain.
    static func plainCredentialHeader(in headers: [WebhookHeader]) -> WebhookHeader? {
        headers.first {
            WebhookHeaderPolicy.looksSensitive(name: $0.name)
                && !$0.value.trimmingCharacters(in: .whitespaces).isEmpty
                && !WebhookSecretPlaceholder.contains($0.value)
        }
    }

    /// Moves such a header's value into the secret: `Bearer abc` becomes secret `abc` and value
    /// `Bearer {{secret}}`; anything else moves whole.
    static func movingToSecret(_ header: WebhookHeader) -> (secret: String, value: String) {
        let value = header.value.trimmingCharacters(in: .whitespaces)
        for prefix in ["Bearer ", "Token ", "Basic "] where value.lowercased().hasPrefix(prefix.lowercased()) {
            let secret = String(value.dropFirst(prefix.count)).trimmingCharacters(in: .whitespaces)
            return (secret, prefix + WebhookSecretPlaceholder.token)
        }
        return (value, WebhookSecretPlaceholder.token)
    }
}
