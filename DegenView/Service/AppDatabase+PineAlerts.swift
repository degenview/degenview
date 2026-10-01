import Foundation
import GRDB

/// Pine script alert storage: the subscriptions, and the history of what was delivered.
extension AppDatabase {
    /// History rows kept; older ones are pruned on insert.
    static let pineAlertHistoryLimit = 500

    func pineAlertSubscriptions() -> [PineAlertSubscription] {
        documents(PineAlertSubscription.self, in: .pineAlertSubscription)
    }

    func savePineAlertSubscriptions(_ subscriptions: [PineAlertSubscription]) {
        replaceDocuments(subscriptions, in: .pineAlertSubscription)
    }

    /// Records a delivery. Returns false, and stores nothing, when `dedupeKey` was already recorded:
    /// that bar was delivered before, possibly by an earlier launch.
    @discardableResult
    func insertPineAlertEvent(_ notification: PineAlertNotification, dedupeKey: String?) -> Bool {
        do {
            return try writer.write { db in
                try db.execute(
                    sql: """
                        INSERT OR IGNORE INTO pine_alert_event (id, subscription_id, timestamp, dedupe_key, payload)
                        VALUES (?, ?, ?, ?, ?)
                        """,
                    arguments: [
                        notification.id.uuidString, notification.subscriptionID.uuidString,
                        notification.triggeredAt.timeIntervalSince1970, dedupeKey, try Self.json(notification),
                    ])
                guard db.changesCount > 0 else { return false }
                try db.execute(
                    sql: """
                        DELETE FROM pine_alert_event WHERE id NOT IN (
                            SELECT id FROM pine_alert_event ORDER BY timestamp DESC, rowid DESC LIMIT ?)
                        """,
                    arguments: [Self.pineAlertHistoryLimit])
                return true
            }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not record Pine alert: \(error.localizedDescription)")
            #endif
            return false
        }
    }

    /// Newest first.
    func recentPineAlertEvents(limit: Int = 100) -> [PineAlertNotification] {
        do {
            return try reader.read { db in
                try String.fetchAll(
                    db, sql: "SELECT payload FROM pine_alert_event ORDER BY timestamp DESC, rowid DESC LIMIT ?",
                    arguments: [limit]
                ).compactMap { try? Self.decoder.decode(PineAlertNotification.self, from: Data($0.utf8)) }
            }
        } catch {
            return []
        }
    }

    /// Keys of recent deliveries, to seed `PineAlertFrequencyGuard` after a relaunch.
    func recentPineAlertDedupeKeys() -> [String] {
        (try? reader.read { db in
            try String.fetchAll(
                db, sql: "SELECT dedupe_key FROM pine_alert_event WHERE dedupe_key IS NOT NULL ORDER BY timestamp")
        }) ?? []
    }

    /// A re-armed subscription may deliver the bar it last delivered for, so its history stops
    /// blocking it. The rows stay as history.
    func clearPineAlertDedupeKeys(subscriptionID: UUID) {
        try? writer.write { db in
            try db.execute(
                sql: "UPDATE pine_alert_event SET dedupe_key = NULL WHERE subscription_id = ?",
                arguments: [subscriptionID.uuidString])
        }
    }

    func deletePineAlertEvents(subscriptionID: UUID) {
        try? writer.write { db in
            try db.execute(
                sql: "DELETE FROM pine_alert_event WHERE subscription_id = ?", arguments: [subscriptionID.uuidString])
        }
    }
}
