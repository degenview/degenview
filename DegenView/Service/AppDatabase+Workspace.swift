import Foundation
import GRDB

/// Tabs, saved views, and drawings. App-only: the alert agent never touches workspace state.
extension AppDatabase {

    // MARK: - Saved views

    func savedViews() -> [SavedView] {
        documents(SavedView.self, in: .savedView)
    }

    func saveSavedViews(_ views: [SavedView]) {
        replaceDocuments(views, in: .savedView)
    }

    // MARK: - Tabs

    /// Nil when no tab has ever been saved.
    func tabsSnapshot() throws -> TabsSnapshot? {
        try reader.read { db in
            let tabs = try Self.documents(ChartTab.self, in: .tab, db: db)
            guard !tabs.isEmpty else { return nil }
            let groups = try String.fetchAll(db, sql: "SELECT tab_ids FROM window_group ORDER BY position")
                .compactMap { try? Self.decoder.decode([UUID].self, from: Data($0.utf8)) }
            return TabsSnapshot(tabs: tabs, windowGroups: groups)
        }
    }

    func saveTabs(_ snapshot: TabsSnapshot) throws {
        try writer.write { try Self.replaceTabs(snapshot, db: $0) }
    }

    private static func replaceTabs(_ snapshot: TabsSnapshot, db: Database) throws {
        try replaceDocuments(snapshot.tabs, in: .tab, db: db)
        try db.execute(sql: "DELETE FROM window_group")
        for (position, group) in snapshot.windowGroups.enumerated() {
            try db.execute(
                sql: "INSERT INTO window_group (position, tab_ids) VALUES (?, ?)",
                arguments: [position, try json(group)])
        }
    }

    // MARK: - Drawings

    enum DrawingKind: String {
        case trendLine = "trend_line"
        case fibonacci
    }

    /// Drawings of `kind` for every instrument that has any.
    func drawings<T: Decodable>(_ type: T.Type, kind: DrawingKind) -> [String: [T]] {
        do {
            return try reader.read { db in
                var result: [String: [T]] = [:]
                let rows = try Row.fetchAll(
                    db,
                    sql: "SELECT instrument, payload FROM drawing WHERE kind = ? ORDER BY instrument, position",
                    arguments: [kind.rawValue])
                for row in rows {
                    let payload: String = row["payload"]
                    guard let value = try? Self.decoder.decode(T.self, from: Data(payload.utf8)) else { continue }
                    result[row["instrument"], default: []].append(value)
                }
                return result
            }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not read drawings: \(error.localizedDescription)")
            #endif
            return [:]
        }
    }

    func saveDrawings<T: Encodable & Identifiable>(_ items: [T], instrument: String, kind: DrawingKind)
    where T.ID == UUID {
        do {
            try writer.write { try Self.replaceDrawings(items, instrument: instrument, kind: kind, db: $0) }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not write drawings: \(error.localizedDescription)")
            #endif
        }
    }

    private static func replaceDrawings<T: Encodable & Identifiable>(
        _ items: [T], instrument: String, kind: DrawingKind, db: Database
    ) throws where T.ID == UUID {
        try db.execute(
            sql: "DELETE FROM drawing WHERE instrument = ? AND kind = ?", arguments: [instrument, kind.rawValue])
        let statement = try db.makeStatement(
            sql: """
                INSERT OR REPLACE INTO drawing (instrument, kind, id, position, payload)
                VALUES (?, ?, ?, ?, ?)
                """)
        for (position, item) in items.enumerated() {
            try statement.execute(arguments: [
                instrument, kind.rawValue, item.id.uuidString, position, try json(item),
            ])
        }
    }
}
