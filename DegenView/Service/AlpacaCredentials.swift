import Foundation
import Security

struct AlpacaCredentials: Codable, Equatable {
    var keyID: String
    var secretKey: String

    var isConfigured: Bool {
        !keyID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !secretKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

enum AlpacaCredentialsStore {
    private static let service = "com.cryptocharts.alpaca"
    private static let credentialsAccount = "credentials-v1"
    private static var accessGroup: String? {
        guard let task = SecTaskCreateFromSelf(nil),
              let groups = SecTaskCopyValueForEntitlement(task, "keychain-access-groups" as CFString, nil) as? [String]
        else { return nil }
        return groups.first { $0.hasSuffix("com.cryptocharts.shared") }
    }
    private static let lock = NSLock()
    private static var cachedCredentials: AlpacaCredentials?

    static var credentials: AlpacaCredentials {
        lock.lock()
        defer { lock.unlock() }

        if let cachedCredentials { return cachedCredentials }

        if let data = readData(account: credentialsAccount, shared: true) ?? readData(account: credentialsAccount, shared: false),
           let stored = try? JSONDecoder().decode(AlpacaCredentials.self, from: data) {
            cachedCredentials = stored
            try? writeData(data, account: credentialsAccount)
            return stored
        }

        let empty = AlpacaCredentials(keyID: "", secretKey: "")
        cachedCredentials = empty
        return empty
    }

    static var isConfigured: Bool { credentials.isConfigured }

    static func save(_ credentials: AlpacaCredentials) throws {
        let normalized = AlpacaCredentials(
            keyID: credentials.keyID.trimmingCharacters(in: .whitespacesAndNewlines),
            secretKey: credentials.secretKey.trimmingCharacters(in: .whitespacesAndNewlines)
        )

        lock.lock()
        do {
            if normalized.isConfigured {
                try writeData(JSONEncoder().encode(normalized), account: credentialsAccount)
            } else {
                delete(account: credentialsAccount)
            }
            cachedCredentials = normalized
            lock.unlock()
        } catch {
            lock.unlock()
            throw error
        }
        NotificationCenter.default.post(name: .alpacaCredentialsChanged, object: nil)
    }

    private static func readData(account: String, shared: Bool) -> Data? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        if shared, let accessGroup { query[kSecAttrAccessGroup as String] = accessGroup }
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data else { return nil }
        return data
    }

    private static func writeData(_ data: Data, account: String) throws {
        var key: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        if let accessGroup { key[kSecAttrAccessGroup as String] = accessGroup }
        let status = SecItemUpdate(
            key as CFDictionary,
            [kSecValueData as String: data] as CFDictionary
        )
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

    private static func delete(account: String) {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        if let accessGroup { query[kSecAttrAccessGroup as String] = accessGroup }
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { return }
    }
}

extension Notification.Name {
    static let alpacaCredentialsChanged = Notification.Name("AlpacaCredentialsChanged")
}
