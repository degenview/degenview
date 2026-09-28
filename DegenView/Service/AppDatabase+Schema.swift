import Foundation
import GRDB

extension AppDatabase {
    /// Append-only: never edit a registered migration once it has shipped, add a new one.
    static var migrator: DatabaseMigrator {
        var migrator = DatabaseMigrator()

        migrator.registerMigration("v1_documents") { db in
            // Ordered lists of Codable values. The payload stays JSON so nested chart
            // configuration can evolve through Codable defaults instead of migrations.
            for table in DocumentTable.allCases {
                try db.create(table: table.rawValue) { t in
                    t.primaryKey("id", .text)
                    t.column("position", .integer).notNull()
                    t.column("payload", .text).notNull()
                }
            }

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
            // An instrument whose drawings were all deleted still has a row here, so a
            // legacy saved view can't re-import lines the user already removed.
            try db.create(table: "drawing_instrument") { t in
                t.column("instrument", .text).notNull()
                t.column("kind", .text).notNull()
                t.primaryKey(["instrument", "kind"])
            }
        }

        return migrator
    }
}

// MARK: - Ordered documents

/// Tables holding one ordered list of Codable values, keyed by the value's UUID.
enum DocumentTable: String, CaseIterable {
    case favorite
    case savedView = "saved_view"
    case tab
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
