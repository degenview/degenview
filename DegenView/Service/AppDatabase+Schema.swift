import Foundation
import GRDB

extension AppDatabase {
    /// Append-only: never edit a registered migration once it has shipped, add a new one.
    static var migrator: DatabaseMigrator {
        var migrator = DatabaseMigrator()

        migrator.registerMigration("v1") { db in
            // Ordered lists of Codable values. The payload stays JSON so nested chart
            // configuration can evolve through Codable defaults instead of migrations.
            for table in ["favorite", "saved_view", "tab", "portfolio", "paper_account", "price_alert"] {
                try createDocumentTable(table, db: db)
            }

            try db.create(table: "setting") { t in
                t.primaryKey("key", .text)
                t.column("value", .text).notNull()
            }

            // MARK: Workspace

            try db.create(table: "window_group") { t in
                t.primaryKey("position", .integer)
                t.column("tab_ids", .text).notNull()
            }

            try db.create(table: "drawing") { t in
                t.column("instrument", .text).notNull()
                t.column("kind", .text).notNull()
                t.column("id", .text).notNull()
                t.column("position", .integer).notNull()
                t.column("payload", .text).notNull()
                t.primaryKey(["instrument", "kind", "id"])
            }

            // MARK: Portfolio

            // Transaction order is significant to validation, so `position` preserves it.
            try db.create(table: "portfolio_transaction") { t in
                t.primaryKey("id", .text)
                t.column("portfolio_id", .text).notNull()
                t.column("timestamp", .double).notNull()
                t.column("position", .integer).notNull()
                t.column("payload", .text).notNull()
            }
            try db.create(
                index: "portfolio_transaction_on_portfolio", on: "portfolio_transaction",
                columns: ["portfolio_id", "timestamp"])

            try db.create(table: "portfolio_snapshot") { t in
                t.autoIncrementedPrimaryKey("rowid")
                t.column("portfolio_id", .text).notNull()
                t.column("timestamp", .double).notNull()
                t.column("payload", .text).notNull()
            }
            try db.create(
                index: "portfolio_snapshot_on_portfolio", on: "portfolio_snapshot",
                columns: ["portfolio_id", "timestamp"])

            // MARK: Paper trading

            // Collections owned by one account, kept in engine order via the rowid.
            for table in [
                "paper_order", "paper_fill", "paper_position", "paper_order_event",
                "paper_closed_trade", "paper_journal",
            ] {
                try db.create(table: table) { t in
                    t.autoIncrementedPrimaryKey("rowid")
                    t.column("account_id", .text).notNull()
                    t.column("payload", .text).notNull()
                }
                try db.create(index: "\(table)_on_account", on: table, columns: ["account_id"])
            }

            // MARK: Alerts

            try db.create(table: "alert_event") { t in
                t.autoIncrementedPrimaryKey("rowid")
                t.column("alert_id", .text).notNull()
                t.column("timestamp", .double).notNull()
                t.column("payload", .text).notNull()
            }
            try db.create(index: "alert_event_on_alert", on: "alert_event", columns: ["alert_id"])

            // GUI → runtime queue. The runtime deletes a row once the command is applied.
            try db.create(table: "alert_command") { t in
                t.primaryKey("id", .text)
                t.column("created_at", .double).notNull()
                t.column("payload", .text).notNull()
            }
            try db.create(index: "alert_command_on_created_at", on: "alert_command", columns: ["created_at"])
        }

        return migrator
    }

    private static func createDocumentTable(_ name: String, db: Database) throws {
        try db.create(table: name) { t in
            t.primaryKey("id", .text)
            t.column("position", .integer).notNull()
            t.column("payload", .text).notNull()
        }
    }
}

// MARK: - Ordered documents

/// Tables holding one ordered list of Codable values, keyed by the value's UUID.
enum DocumentTable: String {
    case favorite
    case savedView = "saved_view"
    case tab
    case portfolio
    case paperAccount = "paper_account"
    case priceAlert = "price_alert"
}

extension AppDatabase {
    static let encoder = JSONEncoder()
    static let decoder = JSONDecoder()

    /// Rows that no longer decode are skipped rather than failing the whole list.
    func documents<T: Decodable>(_ type: T.Type, in table: DocumentTable) -> [T] {
        do {
            return try reader.read { try Self.documents(type, in: table, db: $0) }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not read \(table.rawValue): \(error.localizedDescription)")
            #endif
            return []
        }
    }

    func replaceDocuments<T: Encodable & Identifiable>(_ items: [T], in table: DocumentTable)
    where T.ID == UUID {
        do {
            try writer.write { try Self.replaceDocuments(items, in: table, db: $0) }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not write \(table.rawValue): \(error.localizedDescription)")
            #endif
        }
    }

    static func documents<T: Decodable>(_ type: T.Type, in table: DocumentTable, db: Database) throws -> [T] {
        try String.fetchAll(db, sql: "SELECT payload FROM \(table.rawValue) ORDER BY position")
            .compactMap { payload in
                do {
                    return try decoder.decode(T.self, from: Data(payload.utf8))
                } catch {
                    #if DEBUG
                        print("[AppDatabase] Skipping undecodable \(table.rawValue) row: \(error)")
                    #endif
                    return nil
                }
            }
    }

    static func replaceDocuments<T: Encodable & Identifiable>(
        _ items: [T], in table: DocumentTable, db: Database
    ) throws where T.ID == UUID {
        try db.execute(sql: "DELETE FROM \(table.rawValue)")
        let statement = try db.makeStatement(
            sql: "INSERT OR REPLACE INTO \(table.rawValue) (id, position, payload) VALUES (?, ?, ?)")
        for (position, item) in items.enumerated() {
            try statement.execute(arguments: [item.id.uuidString, position, try json(item)])
        }
    }

    static func json<T: Encodable>(_ value: T) throws -> String {
        String(decoding: try encoder.encode(value), as: UTF8.self)
    }
}

// MARK: - Settings

/// Single Codable values that don't warrant a table, keyed by a dotted name.
extension AppDatabase {
    static func setting<T: Decodable>(_ type: T.Type, key: String, db: Database) throws -> T? {
        guard let value = try String.fetchOne(db, sql: "SELECT value FROM setting WHERE key = ?", arguments: [key])
        else { return nil }
        return try decoder.decode(T.self, from: Data(value.utf8))
    }

    /// Nil deletes the key.
    static func setSetting<T: Encodable>(_ value: T?, key: String, db: Database) throws {
        guard let value else {
            try db.execute(sql: "DELETE FROM setting WHERE key = ?", arguments: [key])
            return
        }
        try db.execute(
            sql: "INSERT OR REPLACE INTO setting (key, value) VALUES (?, ?)", arguments: [key, try json(value)])
    }

    func setting<T: Decodable>(_ type: T.Type, key: String) -> T? {
        do {
            return try reader.read { try Self.setting(type, key: key, db: $0) }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not read setting \(key): \(error.localizedDescription)")
            #endif
            return nil
        }
    }

    func setSetting<T: Encodable>(_ value: T?, key: String) {
        do {
            try writer.write { try Self.setSetting(value, key: key, db: $0) }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not write setting \(key): \(error.localizedDescription)")
            #endif
        }
    }
}
