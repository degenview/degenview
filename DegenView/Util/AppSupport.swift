import Foundation

/// Centralized Application Support directory and shared URLSession configuration.
enum AppSupport {

    /// DegenView directory in Application Support — created lazily.
    static let directory: URL = {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!
        let fm = FileManager.default
        let dir = appSupport.appendingPathComponent("DegenView", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }()

    /// Shared URLSession with standard timeouts for API calls.
    static let defaultSession: URLSession = {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = Timeout.request
        config.timeoutIntervalForResource = Timeout.resource
        return URLSession(configuration: config)
    }()
}
