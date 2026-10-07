import Foundation

/// The outcome of one successful look at the release feed.
enum UpdateCheckResult: Equatable {
    case upToDate
    case available(version: String, releaseURL: URL)
}

/// Asks where the running version stands. The real implementation (release feed lookup) replaces
/// `PlaceholderUpdateChecker` in `UpdateCheckViewModel`'s default; nothing else changes.
protocol UpdateChecker: Sendable {
    func check(currentVersion: String) async throws -> UpdateCheckResult
}
