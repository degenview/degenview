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
