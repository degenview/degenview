import Foundation
import ServiceManagement

@MainActor
final class AlertBackgroundService {
    static let shared = AlertBackgroundService()
    private let service = SMAppService.loginItem(identifier: "com.cryptocharts.app.alert-agent")

    var state: AlertServiceState {
        switch service.status {
        case .enabled: .enabled
        case .requiresApproval: .requiresApproval
        case .notFound: .notFound
        case .notRegistered: .disabled
        @unknown default: .unavailable
        }
    }

    @discardableResult func register() -> AlertServiceState {
        do { try service.register() } catch { return .failed }
        return state
    }
}
