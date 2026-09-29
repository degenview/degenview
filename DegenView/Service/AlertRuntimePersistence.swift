import Foundation
import GRDB

/// Cross-process storage for the GUI and login-item agent. Both open the same WAL
/// database: the runtime that owns `alert_runtime.lock` writes the snapshot, the GUI
/// enqueues commands, and each write is a single transaction so readers never see a
/// half-saved snapshot.
struct AlertRuntimePersistence: Sendable {
    static let shared = AlertRuntimePersistence()
    /// Holds `alert_runtime.lock`, which decides which process evaluates alerts.
    let directory: URL
    private let database: AppDatabase

    private enum Key {
        static let schemaVersion = "alert.schemaVersion"
        static let revision = "alert.revision"
        static let settings = "alert.settings"
        static let processedCommandIDs = "alert.processedCommandIDs"
        static let health = "alert.health"
    }

    init(database: AppDatabase = .shared, directory: URL = AppSupport.directory) {
        self.database = database
        self.directory = directory
    }

    /// Nil until a runtime has saved once, so the engine can start from defaults.
    func loadSnapshot() -> AlertPersistenceSnapshot? {
        do {
            return try database.reader.read { db in
                guard let schemaVersion = try AppDatabase.setting(Int.self, key: Key.schemaVersion, db: db)
                else { return nil }
                var snapshot = AlertPersistenceSnapshot()
                snapshot.schemaVersion = schemaVersion
                snapshot.revision = try AppDatabase.setting(UInt64.self, key: Key.revision, db: db) ?? 0
                // Per-row decoding: one unreadable alert or event must not make the engine
                // start empty and overwrite the rest.
                snapshot.alerts = try AppDatabase.documents(PriceAlert.self, in: .priceAlert, db: db)
                snapshot.history = try String.fetchAll(db, sql: "SELECT payload FROM alert_event ORDER BY rowid")
                    .compactMap { try? AppDatabase.decoder.decode(AlertTriggerEvent.self, from: Data($0.utf8)) }
                snapshot.settings =
                    (try? AppDatabase.setting(AlertNotificationSettings.self, key: Key.settings, db: db))
                    ?? AlertNotificationSettings()
                snapshot.processedCommandIDs =
                    (try? AppDatabase.setting([UUID].self, key: Key.processedCommandIDs, db: db)) ?? []
                snapshot.health =
                    (try? AppDatabase.setting(AlertRuntimeHealth.self, key: Key.health, db: db))
                    ?? AlertRuntimeHealth()
                return snapshot
            }
        } catch {
            #if DEBUG
                print("[AlertRuntimePersistence] Could not load alerts: \(error.localizedDescription)")
            #endif
            return nil
        }
    }

    func saveSnapshot(_ snapshot: AlertPersistenceSnapshot) {
        do {
            try database.writer.write { try Self.replaceSnapshot(snapshot, db: $0) }
        } catch {
            #if DEBUG
                print("[AlertRuntimePersistence] Could not save alerts: \(error.localizedDescription)")
            #endif
        }
    }

    func enqueue(_ command: AlertRuntimeCommand) throws {
        try database.writer.write { try Self.insert(command, db: $0) }
    }

    /// Oldest first. Rows that no longer decode stay queued rather than being dropped.
    func pendingCommands() -> [AlertRuntimeCommand] {
        let payloads =
            (try? database.reader.read { db in
                try String.fetchAll(db, sql: "SELECT payload FROM alert_command ORDER BY created_at, rowid")
            }) ?? []
        return payloads.compactMap { try? AppDatabase.decoder.decode(AlertRuntimeCommand.self, from: Data($0.utf8)) }
    }

    func acknowledge(_ id: UUID) {
        try? database.writer.write {
            try $0.execute(sql: "DELETE FROM alert_command WHERE id = ?", arguments: [id.uuidString])
        }
    }

    // MARK: - Private

    private static func replaceSnapshot(_ snapshot: AlertPersistenceSnapshot, db: Database) throws {
        try AppDatabase.replaceDocuments(snapshot.alerts, in: .priceAlert, db: db)
        try db.execute(sql: "DELETE FROM alert_event")
        let event = try db.makeStatement(
            sql: "INSERT INTO alert_event (alert_id, timestamp, payload) VALUES (?, ?, ?)")
        for value in snapshot.history {
            try event.execute(arguments: [
                value.alertID.uuidString, value.timestamp.timeIntervalSince1970, try AppDatabase.json(value),
            ])
        }
        try AppDatabase.setSetting(snapshot.schemaVersion, key: Key.schemaVersion, db: db)
        try AppDatabase.setSetting(snapshot.revision, key: Key.revision, db: db)
        try AppDatabase.setSetting(snapshot.settings, key: Key.settings, db: db)
        try AppDatabase.setSetting(snapshot.processedCommandIDs, key: Key.processedCommandIDs, db: db)
        try AppDatabase.setSetting(snapshot.health, key: Key.health, db: db)
    }

    private static func insert(_ command: AlertRuntimeCommand, db: Database) throws {
        try db.execute(
            sql: "INSERT OR IGNORE INTO alert_command (id, created_at, payload) VALUES (?, ?, ?)",
            arguments: [
                command.id.uuidString, command.createdAt.timeIntervalSince1970, try AppDatabase.json(command),
            ])
    }
}
