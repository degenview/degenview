import Foundation

/// Centralized Application Support directory and shared URLSession configuration.
enum AppSupport {

    /// CryptoCharts directory in Application Support — created lazily.
    static let directory: URL = {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!
        let dir = appSupport.appendingPathComponent("CryptoCharts", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
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
