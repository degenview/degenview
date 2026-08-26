import Foundation

/// Cross-process storage for the GUI and login-item agent. Every access is coordinated;
/// writes use Foundation's atomic replacement so readers never observe partial JSON.
struct AlertRuntimePersistence: Sendable {
    static let shared = AlertRuntimePersistence()
    let directory: URL
    let snapshotURL: URL
    let commandsURL: URL

    init(directory: URL = AppSupport.directory) {
        self.directory = directory
        snapshotURL = directory.appendingPathComponent("price_alerts.json")
        commandsURL = directory.appendingPathComponent("alert_commands", isDirectory: true)
        try? FileManager.default.createDirectory(at: commandsURL, withIntermediateDirectories: true)
    }

    func loadSnapshot() -> AlertPersistenceSnapshot? {
        guard FileManager.default.fileExists(atPath: snapshotURL.path) else { return nil }
        var result: AlertPersistenceSnapshot?
        var coordinationError: NSError?
        NSFileCoordinator().coordinate(readingItemAt: snapshotURL, options: [], error: &coordinationError) { url in
            guard let data = try? Data(contentsOf: url) else { return }
            result = try? Self.decoder.decode(AlertPersistenceSnapshot.self, from: data)
            if result == nil { quarantineMalformedFile(url) }
        }
        return result
    }

    func saveSnapshot(_ snapshot: AlertPersistenceSnapshot) {
        guard let data = try? Self.encoder.encode(snapshot) else { return }
        var coordinationError: NSError?
        NSFileCoordinator().coordinate(writingItemAt: snapshotURL, options: .forReplacing, error: &coordinationError) {
            url in
            try? data.write(to: url, options: .atomic)
        }
    }

    func enqueue(_ command: AlertRuntimeCommand) throws {
        let url = commandsURL.appendingPathComponent(
            "\(command.createdAt.timeIntervalSince1970)-\(command.id.uuidString).json")
        let data = try Self.encoder.encode(command)
        var coordinationError: NSError?
        NSFileCoordinator().coordinate(writingItemAt: url, options: .forReplacing, error: &coordinationError) {
            coordinatedURL in
            try? data.write(to: coordinatedURL, options: .atomic)
        }
        if let coordinationError { throw coordinationError }
    }

    func pendingCommands() -> [(URL, AlertRuntimeCommand)] {
        let urls =
            (try? FileManager.default.contentsOfDirectory(at: commandsURL, includingPropertiesForKeys: nil)) ?? []
        return urls.filter { $0.pathExtension == "json" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
            .compactMap { url in
                var command: AlertRuntimeCommand?
                var error: NSError?
                NSFileCoordinator().coordinate(readingItemAt: url, options: [], error: &error) { coordinatedURL in
                    guard let data = try? Data(contentsOf: coordinatedURL) else { return }
                    command = try? Self.decoder.decode(AlertRuntimeCommand.self, from: data)
                }
                return command.map { (url, $0) }
            }
    }

    func acknowledge(_ url: URL) {
        var error: NSError?
        NSFileCoordinator().coordinate(writingItemAt: url, options: .forDeleting, error: &error) {
            try? FileManager.default.removeItem(at: $0)
        }
    }

    private func quarantineMalformedFile(_ url: URL) {
        let suffix = ISO8601DateFormatter().string(from: Date()).replacingOccurrences(of: ":", with: "-")
        try? FileManager.default.copyItem(
            at: url, to: directory.appendingPathComponent("price_alerts.malformed-\(suffix).json"))
    }

    private static let encoder: JSONEncoder = {
        let value = JSONEncoder()
        value.outputFormatting = [.prettyPrinted, .sortedKeys]
        return value
    }()
    private static let decoder = JSONDecoder()
}
