import Foundation
import GRDB

/// Webhook endpoints and the history of delivery attempts. Compiled into the login-item agent too:
/// it reads endpoints and records price-alert deliveries.
extension AppDatabase {
    /// Delivery rows kept; older ones are pruned when a new attempt is claimed.
    static let webhookDeliveryHistoryLimit = 5000

    // MARK: Endpoints

    /// Throws when the table cannot be read, or when any row fails to decode. Unlike `documents`,
    /// it never hands back a shorter list: an empty or partial list must not be mistaken for "no
    /// endpoints" by a caller that then writes it back.
    func webhookEndpoints() throws -> [WebhookEndpoint] {
        try reader.read { db in
            try String.fetchAll(db, sql: "SELECT payload FROM webhook_endpoint ORDER BY position")
                .map { try Self.decoder.decode(WebhookEndpoint.self, from: Data($0.utf8)) }
        }
    }

    func replaceWebhookEndpoints(_ endpoints: [WebhookEndpoint]) throws {
        try replaceDocumentsThrowing(endpoints, in: .webhookEndpoint)
    }

    // MARK: Deliveries

    /// Claims the right to send `eventID` to `endpointID`. Returns the new row id, or nil when that
    /// trigger already reached the endpoint (or is being sent): whoever gets nil must not send.
    /// The row is written before the request, so an owner that dies mid-request leaves a `pending`
    /// row behind and the next owner does not send it again.
    func claimWebhookDelivery(
        endpointID: UUID, source: WebhookDeliverySource, eventID: UUID, now: Date = Date()
    ) -> UUID? {
        let id = UUID()
        do {
            return try writer.write { db in
                try db.execute(
                    sql: """
                        INSERT OR IGNORE INTO webhook_delivery
                            (id, endpoint_id, source, event_id, timestamp, state)
                        VALUES (?, ?, ?, ?, ?, ?)
                        """,
                    arguments: [
                        id.uuidString, endpointID.uuidString, source.rawValue, eventID.uuidString,
                        now.timeIntervalSince1970, WebhookDeliveryState.pending.rawValue,
                    ])
                guard db.changesCount > 0 else { return nil }
                try db.execute(
                    sql: """
                        DELETE FROM webhook_delivery WHERE id NOT IN (
                            SELECT id FROM webhook_delivery ORDER BY timestamp DESC, rowid DESC LIMIT ?)
                        """,
                    arguments: [Self.webhookDeliveryHistoryLimit])
                return id
            }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not claim webhook delivery: \(error.localizedDescription)")
            #endif
            return nil
        }
    }

    /// Records how a claimed attempt ended.
    func finishWebhookDelivery(id: UUID, result: WebhookDeliveryResult) {
        do {
            try writer.write { db in
                try db.execute(
                    sql: """
                        UPDATE webhook_delivery
                        SET state = ?, status_code = ?, duration = ?, error = ?, timestamp = ?
                        WHERE id = ?
                        """,
                    arguments: [
                        (result.succeeded ? WebhookDeliveryState.delivered : .failed).rawValue,
                        result.statusCode, result.duration, result.error?.rawValue,
                        result.timestamp.timeIntervalSince1970, id.uuidString,
                    ])
            }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not finish webhook delivery: \(error.localizedDescription)")
            #endif
        }
    }

    /// Attempts for the given triggers, oldest first within each.
    func webhookDeliveries(eventIDs: [UUID]) -> [UUID: [WebhookDeliveryRecord]] {
        guard !eventIDs.isEmpty else { return [:] }
        do {
            return try reader.read { db in
                var grouped: [UUID: [WebhookDeliveryRecord]] = [:]
                // SQLite caps bound variables, so large lists go in chunks.
                for start in stride(from: 0, to: eventIDs.count, by: 500) {
                    let chunk = eventIDs[start..<min(start + 500, eventIDs.count)]
                    let marks = Array(repeating: "?", count: chunk.count).joined(separator: ",")
                    let rows = try Row.fetchAll(
                        db,
                        sql: """
                            SELECT * FROM webhook_delivery WHERE event_id IN (\(marks))
                            ORDER BY timestamp, rowid
                            """,
                        arguments: StatementArguments(chunk.map(\.uuidString)))
                    for row in rows {
                        guard let record = Self.webhookDeliveryRecord(row), let eventID = record.eventID else {
                            continue
                        }
                        grouped[eventID, default: []].append(record)
                    }
                }
                return grouped
            }
        } catch {
            return [:]
        }
    }

    private static func webhookDeliveryRecord(_ row: Row) -> WebhookDeliveryRecord? {
        guard let id = UUID(uuidString: row["id"]), let endpointID = UUID(uuidString: row["endpoint_id"]),
            let source = WebhookDeliverySource(rawValue: row["source"]),
            let state = WebhookDeliveryState(rawValue: row["state"])
        else { return nil }
        let eventString: String? = row["event_id"]
        let errorString: String? = row["error"]
        return WebhookDeliveryRecord(
            id: id, endpointID: endpointID, source: source, eventID: eventString.flatMap(UUID.init(uuidString:)),
            timestamp: Date(timeIntervalSince1970: row["timestamp"]), state: state,
            statusCode: row["status_code"], duration: row["duration"],
            error: errorString.flatMap(WebhookDeliveryError.init(rawValue:)))
    }
}
