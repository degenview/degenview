import Foundation

/// Off switch for every Keychain read and write. Under XCTest, or with `DEGENVIEW_NO_KEYCHAIN=1`
/// in the environment, credential stores behave as "nothing saved" so a rebuilt, ad hoc signed
/// binary never raises the Keychain access prompt.
enum KeychainPolicy {
    static let isDisabled: Bool = {
        let environment = ProcessInfo.processInfo.environment
        return environment["XCTestConfigurationFilePath"] != nil
            || environment["DEGENVIEW_NO_KEYCHAIN"] == "1"
    }()
}
