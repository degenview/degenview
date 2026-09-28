import Foundation
import GRDB

/// SQLite store for user-authored data: tabs, saved views, favorites, drawings, and
/// (as they migrate) portfolios, paper trading, and alerts. Caches stay in `JSONStore`.
///
/// Opened as a WAL `DatabasePool`, so the GUI and the login-item agent can share the
/// file: readers never block the writer, and a writer waits on `busyTimeout` for the
/// other process instead of failing.
final class AppDatabase: Sendable {
    static let filename = "degenview.sqlite"

    /// Falls back to an in-memory database when the file can't be opened, so a broken
    /// disk degrades to "nothing persists this session" rather than a launch crash.
    static let shared: AppDatabase = {
        do {
            return try AppDatabase(path: AppSupport.directory.appendingPathComponent(filename).path)
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not open \(filename); using memory: \(error.localizedDescription)")
            #endif
            return try! makeInMemory()
        }
    }()

    let writer: any DatabaseWriter

    init(path: String) throws {
        var configuration = Configuration()
        configuration.busyMode = .timeout(5)
        writer = try DatabasePool(path: path, configuration: configuration)
        try Self.migrator.migrate(writer)
    }

    private init(writer: any DatabaseWriter) throws {
        self.writer = writer
        try Self.migrator.migrate(writer)
    }

    static func makeInMemory() throws -> AppDatabase {
        try AppDatabase(writer: DatabaseQueue())
    }

    var reader: any DatabaseReader { writer }
}

// MARK: - Legacy JSON import

extension AppDatabase {
    /// Moves one pre-SQLite JSON document into the database. The file's existence is the
    /// marker: after a committed import it is renamed to `<stem>.migrated.json` and kept as
    /// a manual rollback path. `write` must replace, not append, so an import interrupted
    /// between commit and rename is safe to repeat on the next launch.
    func importLegacyJSON<T: Codable>(
        _ type: T.Type,
        filename: String,
        directory: URL,
        write: (Database, T) throws -> Void
    ) {
        let store = JSONStore<T>(filename: filename, directory: directory)
        guard case .value(let value) = store.loadResult() else { return }
        do {
            try writer.write { try write($0, value) }
            let stem = store.storageURL.deletingPathExtension().lastPathComponent
            var backup = directory.appendingPathComponent("\(stem).migrated.json")
            if FileManager.default.fileExists(atPath: backup.path) {
                backup = directory.appendingPathComponent("\(stem).migrated-\(UUID().uuidString).json")
            }
            try FileManager.default.moveItem(at: store.storageURL, to: backup)
        } catch {
            #if DEBUG
                print("[AppDatabase] Import of \(filename) failed: \(error.localizedDescription)")
            #endif
        }
    }
}
