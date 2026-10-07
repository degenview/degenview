import Foundation
import LocalAuthentication
import Security

/// Where a webhook's secret (an API token or key) lives. The value never goes in SQLite; the
/// endpoint row only holds templates that say where it is used.
protocol WebhookSecretStore: Sendable {
    /// Nil when nothing is stored or it cannot be read. Never prompts.
    func secret(for endpointID: UUID) -> String?
    /// Whether a secret is stored, without reading its value.
    func hasSecret(for endpointID: UUID) -> Bool
    func setSecret(_ secret: String, for endpointID: UUID) throws
    func remove(for endpointID: UUID)
}

/// Keychain-backed store, one generic-password item per endpoint.
///
/// Items go in the shared `com.cryptocharts.shared` access group, which both the app and the
/// login-item alert agent are entitled to, and are read from that group only. The agent has no UI
/// to answer a Keychain prompt, so reads fail instead of asking, and a missing item is simply
/// "unavailable". Without the entitlement (Debug builds) the app can still store and read its own
/// item, but the agent cannot see it, so agent-side price-alert webhooks that use a secret need a
/// Release build.
///
/// Under `KeychainPolicy.isDisabled` (tests, `DEGENVIEW_NO_KEYCHAIN=1`) values stay in memory.
final class KeychainWebhookSecretStore: WebhookSecretStore, @unchecked Sendable {
    static let shared = KeychainWebhookSecretStore()

    private static let service = "com.cryptocharts.webhook"
    private let lock = NSLock()
    private var memory: [UUID: String] = [:]

    private static var accessGroup: String? {
        guard let task = SecTaskCreateFromSelf(nil),
            let groups = SecTaskCopyValueForEntitlement(task, "keychain-access-groups" as CFString, nil) as? [String]
        else { return nil }
        return groups.first { $0.hasSuffix("com.cryptocharts.shared") }
    }

    func secret(for endpointID: UUID) -> String? {
        if KeychainPolicy.isDisabled {
            lock.lock()
            defer { lock.unlock() }
            return memory[endpointID]
        }
        var query = baseQuery(for: endpointID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
            let data = item as? Data
        else { return nil }
        return String(data: data, encoding: .utf8)
    }

    func hasSecret(for endpointID: UUID) -> Bool {
        if KeychainPolicy.isDisabled {
            lock.lock()
            defer { lock.unlock() }
            return memory[endpointID] != nil
        }
        var query = baseQuery(for: endpointID)
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        return status == errSecSuccess || status == errSecInteractionNotAllowed
    }

    func setSecret(_ secret: String, for endpointID: UUID) throws {
        if KeychainPolicy.isDisabled {
            lock.lock()
            memory[endpointID] = secret
            lock.unlock()
            return
        }
        let data = Data(secret.utf8)
        let key = baseQuery(for: endpointID, interactive: true)
        let status = SecItemUpdate(key as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecSuccess { return }
        guard status == errSecItemNotFound else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
        var item = key
        item[kSecValueData as String] = data
        let addStatus = SecItemAdd(item as CFDictionary, nil)
        guard addStatus == errSecSuccess else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(addStatus))
        }
    }

    func remove(for endpointID: UUID) {
        if KeychainPolicy.isDisabled {
            lock.lock()
            memory[endpointID] = nil
            lock.unlock()
            return
        }
        SecItemDelete(baseQuery(for: endpointID, interactive: true) as CFDictionary)
    }

    /// `interactive` lets writes behave as usual; reads never prompt.
    private func baseQuery(for endpointID: UUID, interactive: Bool = false) -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.service,
            kSecAttrAccount as String: "endpoint-\(endpointID.uuidString)",
        ]
        if let group = Self.accessGroup { query[kSecAttrAccessGroup as String] = group }
        if !interactive {
            let context = LAContext()
            context.interactionNotAllowed = true
            query[kSecUseAuthenticationContext as String] = context
        }
        return query
    }
}

/// For tests and previews: nothing touches the Keychain.
final class InMemoryWebhookSecretStore: WebhookSecretStore, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [UUID: String]
    /// Set to make `setSecret` throw, for testing rollback.
    var failsToWrite = false

    init(_ values: [UUID: String] = [:]) { self.values = values }

    func secret(for endpointID: UUID) -> String? {
        lock.lock()
        defer { lock.unlock() }
        return values[endpointID]
    }

    func hasSecret(for endpointID: UUID) -> Bool { secret(for: endpointID) != nil }

    func setSecret(_ secret: String, for endpointID: UUID) throws {
        if failsToWrite { throw NSError(domain: NSOSStatusErrorDomain, code: -1) }
        lock.lock()
        values[endpointID] = secret
        lock.unlock()
    }

    func remove(for endpointID: UUID) {
        lock.lock()
        values[endpointID] = nil
        lock.unlock()
    }
}
