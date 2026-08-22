import Foundation
import Security

struct AlpacaCredentials: Equatable {
    var keyID: String
    var secretKey: String

    var isConfigured: Bool {
        !keyID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !secretKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

enum AlpacaCredentialsStore {
    private static let service = "com.cryptocharts.alpaca"
    private static let keyIDAccount = "key-id"
    private static let secretAccount = "secret-key"

    static var credentials: AlpacaCredentials {
        AlpacaCredentials(keyID: read(keyIDAccount), secretKey: read(secretAccount))
    }

    static var isConfigured: Bool { credentials.isConfigured }

    static func save(_ credentials: AlpacaCredentials) throws {
        try write(credentials.keyID.trimmingCharacters(in: .whitespacesAndNewlines), account: keyIDAccount)
        try write(credentials.secretKey.trimmingCharacters(in: .whitespacesAndNewlines), account: secretAccount)
        NotificationCenter.default.post(name: .alpacaCredentialsChanged, object: nil)
    }

    private static func read(_ account: String) -> String {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data else { return "" }
        return String(data: data, encoding: .utf8) ?? ""
    }

    private static func write(_ value: String, account: String) throws {
        let key: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        SecItemDelete(key as CFDictionary)
        guard !value.isEmpty else { return }
        var item = key
        item[kSecValueData as String] = Data(value.utf8)
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
    }
}

extension Notification.Name {
    static let alpacaCredentialsChanged = Notification.Name("AlpacaCredentialsChanged")
}
