import AppKit
import Foundation
import UserNotifications

@main
enum DegenViewAlertAgentMain {
    static func main() {
        let app = NSApplication.shared
        let delegate = AlertAgentDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.prohibited)
        app.run()
        withExtendedLifetime(delegate) {}
    }
}

private final class AlertAgentDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    private let host = AlertRuntimeHost(role: .agent)
    func applicationDidFinishLaunching(_ notification: Notification) {
        UNUserNotificationCenter.current().delegate = self
        Task { if !(await host.start()) { NSApp.terminate(nil) } }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.cryptocharts.app") else {
            return
        }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        _ = try? await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
    }
}
