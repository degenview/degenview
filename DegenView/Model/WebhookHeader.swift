import Foundation

/// One custom HTTP header on a webhook. The value is a template: `{{secret}}` is replaced with the
/// endpoint's Keychain secret when the request is built.
struct WebhookHeader: Identifiable, Codable, Equatable, Sendable {
    let id: UUID
    var name: String
    var value: String

    init(id: UUID = UUID(), name: String = "", value: String = "") {
        self.id = id
        self.name = name
        self.value = value
    }

    /// A row the user added but never filled in. It is ignored, not an error.
    var isBlank: Bool {
        name.trimmingCharacters(in: .whitespaces).isEmpty && value.trimmingCharacters(in: .whitespaces).isEmpty
    }
}
