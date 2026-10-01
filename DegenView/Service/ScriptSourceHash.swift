import CryptoKit
import Foundation

/// The fingerprint of a script's source: lowercase hex SHA-256. Script records and Pine alert
/// subscriptions use the same one, so "the script is unchanged" means the same thing in both.
enum ScriptSourceHash {
    static func sha256(_ source: String) -> String {
        SHA256.hash(data: Data(source.utf8)).map { String(format: "%02x", $0) }.joined()
    }
}
