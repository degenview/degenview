import Foundation
import GRDB

extension AppDatabase {
    /// Creates every table the app uses. Idempotent, so the GUI and the login-item agent can
    /// both run it against the same file.
    static func createSchema(_ db: Database) throws {
        // Ordered lists of Codable values. The payload stays JSON so nested chart
        // configuration can evolve through Codable defaults instead of schema changes.
        for table in [
            "favorite", "saved_view", "tab", "portfolio", "paper_account", "price_alert", "pine_alert_subscription",
        ] {
            try createDocumentTable(table, db: db)
        }

        try db.create(table: "setting", options: .ifNotExists) { t in
            t.primaryKey("key", .text)
            t.column("value", .text).notNull()
        }

        // MARK: Workspace

        try db.create(table: "window_group", options: .ifNotExists) { t in
            t.primaryKey("position", .integer)
            t.column("tab_ids", .text).notNull()
        }

        try db.create(table: "drawing", options: .ifNotExists) { t in
            t.column("instrument", .text).notNull()
            t.column("kind", .text).notNull()
            t.column("id", .text).notNull()
            t.column("position", .integer).notNull()
            t.column("payload", .text).notNull()
            t.primaryKey(["instrument", "kind", "id"])
        }

        // MARK: Portfolio

        // Transaction order is significant to validation, so `position` preserves it.
        try db.create(table: "portfolio_transaction", options: .ifNotExists) { t in
            t.primaryKey("id", .text)
            t.column("portfolio_id", .text).notNull()
            t.column("timestamp", .double).notNull()
            t.column("position", .integer).notNull()
            t.column("payload", .text).notNull()
        }
        try db.create(
            index: "portfolio_transaction_on_portfolio", on: "portfolio_transaction",
            columns: ["portfolio_id", "timestamp"], options: .ifNotExists)

        try db.create(table: "portfolio_snapshot", options: .ifNotExists) { t in
            t.autoIncrementedPrimaryKey("rowid")
            t.column("portfolio_id", .text).notNull()
            t.column("timestamp", .double).notNull()
            t.column("payload", .text).notNull()
        }
        try db.create(
            index: "portfolio_snapshot_on_portfolio", on: "portfolio_snapshot",
            columns: ["portfolio_id", "timestamp"], options: .ifNotExists)

        // Closed candles never change, so the portfolio keeps them instead of refetching. A
        // disposable cache: dropping both tables only costs a refetch. `covered_from` records
        // how far back a series was fetched, so a hole isn't mistaken for missing history.
        try db.create(table: "candle", options: [.ifNotExists, .withoutRowID]) { t in
            t.column("source", .text).notNull()
            t.column("symbol", .text).notNull()
            t.column("interval", .text).notNull()
            t.column("open_time", .double).notNull()
            t.column("open", .double).notNull()
            t.column("high", .double).notNull()
            t.column("low", .double).notNull()
            t.column("close", .double).notNull()
            t.column("volume", .double).notNull()
            t.column("quote_volume", .double).notNull()
            t.primaryKey(["source", "symbol", "interval", "open_time"])
        }

        try db.create(table: "candle_coverage", options: .ifNotExists) { t in
            t.column("source", .text).notNull()
            t.column("symbol", .text).notNull()
            t.column("interval", .text).notNull()
            t.column("covered_from", .double).notNull()
            t.column("newest", .double).notNull()
            t.primaryKey(["source", "symbol", "interval"])
        }

        // MARK: Paper trading

        // Collections owned by one account, kept in engine order via the rowid.
        for table in [
            "paper_order", "paper_fill", "paper_position", "paper_order_event",
            "paper_closed_trade", "paper_journal",
        ] {
            try db.create(table: table, options: .ifNotExists) { t in
                t.autoIncrementedPrimaryKey("rowid")
                t.column("account_id", .text).notNull()
                t.column("payload", .text).notNull()
            }
            try db.create(index: "\(table)_on_account", on: table, columns: ["account_id"], options: .ifNotExists)
        }

        // MARK: Alerts

        try db.create(table: "alert_event", options: .ifNotExists) { t in
            t.autoIncrementedPrimaryKey("rowid")
            t.column("alert_id", .text).notNull()
            t.column("timestamp", .double).notNull()
            t.column("payload", .text).notNull()
        }
        try db.create(index: "alert_event_on_alert", on: "alert_event", columns: ["alert_id"], options: .ifNotExists)

        // Pine script alerts. `dedupe_key` is unique per call site, bar and mode, so a relaunch
        // cannot deliver the same bar twice; it is NULL for `freq_all`, which may repeat.
        try db.create(table: "pine_alert_event", options: .ifNotExists) { t in
            t.primaryKey("id", .text)
            t.column("subscription_id", .text).notNull()
            t.column("timestamp", .double).notNull()
            t.column("dedupe_key", .text).unique()
            t.column("payload", .text).notNull()
        }
        try db.create(
            index: "pine_alert_event_on_subscription", on: "pine_alert_event",
            columns: ["subscription_id", "timestamp"], options: .ifNotExists)

        // GUI → runtime queue. The runtime deletes a row once the command is applied.
        try db.create(table: "alert_command", options: .ifNotExists) { t in
            t.primaryKey("id", .text)
            t.column("created_at", .double).notNull()
            t.column("payload", .text).notNull()
        }
        try db.create(
            index: "alert_command_on_created_at", on: "alert_command", columns: ["created_at"],
            options: .ifNotExists)
    }

    private static func createDocumentTable(_ name: String, db: Database) throws {
        try db.create(table: name, options: .ifNotExists) { t in
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
    case pineAlertSubscription = "pine_alert_subscription"
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
            try replaceDocumentsThrowing(items, in: table)
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not write \(table.rawValue): \(error.localizedDescription)")
            #endif
        }
    }

    /// For callers that must know the write failed, such as the saved-layout store.
    func replaceDocumentsThrowing<T: Encodable & Identifiable>(_ items: [T], in table: DocumentTable) throws
    where T.ID == UUID {
        try writer.write { try Self.replaceDocuments(items, in: table, db: $0) }
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
