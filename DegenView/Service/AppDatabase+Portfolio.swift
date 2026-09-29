import Foundation
import GRDB

/// Portfolio ledger storage. `PortfolioLedger` still persists whole snapshots, so each
/// save replaces the ledger tables in one transaction.
extension AppDatabase {
    private enum PortfolioKey {
        static let selectedPortfolioID = "portfolio.selectedPortfolioID"
        static let invalidatedAfter = "portfolio.invalidatedAfter"
    }

    func portfolioLedger() throws -> PortfolioLedgerSnapshot {
        try reader.read { db in
            var snapshot = PortfolioLedgerSnapshot()
            snapshot.portfolios = try Self.documents(Portfolio.self, in: .portfolio, db: db)
            snapshot.transactions = try Self.payloads(
                PortfolioTransaction.self, sql: "SELECT payload FROM portfolio_transaction ORDER BY position", db: db)
            snapshot.historicalSnapshots = try Self.payloads(
                PortfolioSnapshot.self, sql: "SELECT payload FROM portfolio_snapshot ORDER BY rowid", db: db)
            snapshot.selectedPortfolioID = try Self.setting(UUID.self, key: PortfolioKey.selectedPortfolioID, db: db)
            snapshot.invalidatedAfter =
                try Self.setting([UUID: Date].self, key: PortfolioKey.invalidatedAfter, db: db) ?? [:]
            return snapshot
        }
    }

    func savePortfolioLedger(_ snapshot: PortfolioLedgerSnapshot) throws {
        try writer.write { try Self.replacePortfolioLedger(snapshot, db: $0) }
    }

    private static func replacePortfolioLedger(_ snapshot: PortfolioLedgerSnapshot, db: Database) throws {
        try replaceDocuments(snapshot.portfolios, in: .portfolio, db: db)

        try db.execute(sql: "DELETE FROM portfolio_transaction")
        let transaction = try db.makeStatement(
            sql: """
                INSERT OR REPLACE INTO portfolio_transaction (id, portfolio_id, timestamp, position, payload)
                VALUES (?, ?, ?, ?, ?)
                """)
        for (position, value) in snapshot.transactions.enumerated() {
            try transaction.execute(arguments: [
                value.id.uuidString, value.portfolioID.uuidString, value.timestamp.timeIntervalSince1970,
                position, try json(value),
            ])
        }

        try db.execute(sql: "DELETE FROM portfolio_snapshot")
        let history = try db.makeStatement(
            sql: "INSERT INTO portfolio_snapshot (portfolio_id, timestamp, payload) VALUES (?, ?, ?)")
        for value in snapshot.historicalSnapshots {
            try history.execute(arguments: [
                value.portfolioID.uuidString, value.timestamp.timeIntervalSince1970, try json(value),
            ])
        }

        try setSetting(snapshot.selectedPortfolioID, key: PortfolioKey.selectedPortfolioID, db: db)
        try setSetting(
            snapshot.invalidatedAfter.isEmpty ? nil : snapshot.invalidatedAfter,
            key: PortfolioKey.invalidatedAfter, db: db)
    }

    /// Unlike `documents`, a row that no longer decodes fails the read: dropping one
    /// transaction would silently change every balance after it.
    private static func payloads<T: Decodable>(_ type: T.Type, sql: String, db: Database) throws -> [T] {
        try String.fetchAll(db, sql: sql).map { try decoder.decode(T.self, from: Data($0.utf8)) }
    }
}
