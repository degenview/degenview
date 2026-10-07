import Foundation

/// Stand-in until update checks exist: waits briefly, like a network call would, then reports
/// the app as current. Never touches the network.
struct PlaceholderUpdateChecker: UpdateChecker {
    var delay: Duration = .milliseconds(900)

    func check(currentVersion: String) async throws -> UpdateCheckResult {
        try await Task.sleep(for: delay)
        return .upToDate
    }
}
