import Foundation

/// Where the app stands against the latest published release.
enum UpdateStatus: Equatable {
    case idle
    case checking
    case upToDate(checkedAt: Date)
    case available(version: String, releaseURL: URL)
    case failed(message: String)
}
