import Foundation

enum WebhookHeaderError: Equatable, Sendable {
    case emptyName
    case invalidName
    case nameTooLong
    case reserved
    case duplicate
    case invalidValue
    case valueTooLong
}

enum WebhookSecretError: Equatable, Sendable {
    case empty
    case tooLong
    case controlCharacters
}

/// What a custom header may be called and say. One place, so the editor, the store and the request
/// builder cannot disagree.
enum WebhookHeaderPolicy {
    static let maximumHeaders = 10
    static let maximumNameLength = 64
    static let maximumValueLength = 2048
    static let maximumSecretLength = 2048

    /// Headers DegenView or URLSession own. Letting a user override them could break the body's
    /// Content-Type detection or the connection itself.
    static let reserved: Set<String> = [
        "content-type", "content-length", "host", "connection", "transfer-encoding", "expect", "upgrade", "te",
        "trailer", "keep-alive",
    ]

    /// RFC 7230 `token` characters.
    private static let tokenCharacters = CharacterSet(
        charactersIn: "!#$%&'*+-.^_`|~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz")

    static func validate(name raw: String) -> WebhookHeaderError? {
        let name = raw.trimmingCharacters(in: .whitespaces)
        if name.isEmpty { return .emptyName }
        if name.count > maximumNameLength { return .nameTooLong }
        if name.unicodeScalars.contains(where: { !tokenCharacters.contains($0) }) { return .invalidName }
        if reserved.contains(name.lowercased()) { return .reserved }
        return nil
    }

    /// Printable ASCII and tabs only: no line breaks (header injection) and nothing a server might
    /// mis-decode. The check is made on the template and again after the secret is in.
    static func validate(value raw: String) -> WebhookHeaderError? {
        let value = raw.trimmingCharacters(in: .whitespaces)
        if value.count > maximumValueLength { return .valueTooLong }
        if value.unicodeScalars.contains(where: { !isHeaderValueScalar($0) }) { return .invalidValue }
        return nil
    }

    static func isHeaderValueScalar(_ scalar: Unicode.Scalar) -> Bool {
        scalar == "\t" || (0x20...0x7E).contains(scalar.value)
    }

    /// The first problem with each row, by header id. Empty means every row is fine.
    static func errors(in headers: [WebhookHeader]) -> [UUID: WebhookHeaderError] {
        var errors: [UUID: WebhookHeaderError] = [:]
        var seen: Set<String> = []
        for header in headers {
            if let problem = validate(name: header.name) ?? validate(value: header.value) {
                errors[header.id] = problem
                continue
            }
            if !seen.insert(header.name.trimmingCharacters(in: .whitespaces).lowercased()).inserted {
                errors[header.id] = .duplicate
            }
        }
        return errors
    }

    static func isValid(_ headers: [WebhookHeader]) -> Bool {
        headers.count <= maximumHeaders && errors(in: headers).isEmpty
    }

    static func validate(secret: String) -> WebhookSecretError? {
        if secret.isEmpty { return .empty }
        if secret.count > maximumSecretLength { return .tooLong }
        if secret.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) {
            return .controlCharacters
        }
        return nil
    }

    // MARK: Wording

    static func message(for error: WebhookHeaderError) -> String {
        switch error {
        case .emptyName: "Enter a header name."
        case .invalidName: "Use letters, numbers and - _ . only."
        case .nameTooLong: "Header names are limited to \(maximumNameLength) characters."
        case .reserved: "DegenView sets this header itself."
        case .duplicate: "This header is already set."
        case .invalidValue: "Use plain letters, numbers and symbols (no line breaks)."
        case .valueTooLong: "Header values are limited to \(maximumValueLength) characters."
        }
    }

    static func message(for error: WebhookSecretError) -> String {
        switch error {
        case .empty: "Enter the secret."
        case .tooLong: "Secrets are limited to \(maximumSecretLength) characters."
        case .controlCharacters: "The secret can't contain line breaks or control characters."
        }
    }

    /// Whether a header of this name normally carries a credential, for the "save it as the secret" nudge.
    static func looksSensitive(name: String) -> Bool {
        let lowered = name.lowercased()
        return ["authorization", "key", "token", "secret", "auth", "password", "bearer"].contains {
            lowered.contains($0)
        }
    }
}
