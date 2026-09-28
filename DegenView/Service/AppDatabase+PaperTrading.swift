import Foundation
import GRDB

/// Paper-trading storage. `PaperTradingEngine` persists whole snapshots, so each save
/// replaces the paper tables in one transaction.
extension AppDatabase {
    private enum PaperKey {
        static let selectedAccountID = "paper.selectedAccountID"
        static let quotes = "paper.quotes"
    }

    func importLegacyPaperTrading(from directory: URL) {
        importLegacyJSON(PaperTradingSnapshot.self, filename: "paper_trading.json", directory: directory) {
            try Self.replacePaperTrading($1, db: $0)
        }
    }

    func paperTrading() throws -> PaperTradingSnapshot {
        try reader.read { db in
            var snapshot = PaperTradingSnapshot()
            snapshot.accounts = try Self.documents(PaperAccount.self, in: .paperAccount, db: db)
            snapshot.orders = try Self.paperRows(PaperOrder.self, table: "paper_order", db: db)
            snapshot.fills = try Self.paperRows(PaperFill.self, table: "paper_fill", db: db)
            snapshot.positions = try Self.paperRows(PaperPosition.self, table: "paper_position", db: db)
            snapshot.orderEvents = try Self.paperRows(PaperOrderEvent.self, table: "paper_order_event", db: db)
            snapshot.closedTrades = try Self.paperRows(PaperClosedTrade.self, table: "paper_closed_trade", db: db)
            snapshot.journal = try Self.paperRows(PaperJournalEntry.self, table: "paper_journal", db: db)
            snapshot.selectedAccountID = try Self.setting(UUID.self, key: PaperKey.selectedAccountID, db: db)
            snapshot.quotes = try Self.setting([String: PaperQuote].self, key: PaperKey.quotes, db: db) ?? [:]
            return snapshot
        }
    }

    func savePaperTrading(_ snapshot: PaperTradingSnapshot) throws {
        try writer.write { try Self.replacePaperTrading(snapshot, db: $0) }
    }

    private static func replacePaperTrading(_ snapshot: PaperTradingSnapshot, db: Database) throws {
        try replaceDocuments(snapshot.accounts, in: .paperAccount, db: db)
        try replacePaperRows(snapshot.orders, table: "paper_order", db: db)
        try replacePaperRows(snapshot.fills, table: "paper_fill", db: db)
        try replacePaperRows(snapshot.positions, table: "paper_position", db: db)
        try replacePaperRows(snapshot.orderEvents, table: "paper_order_event", db: db)
        try replacePaperRows(snapshot.closedTrades, table: "paper_closed_trade", db: db)
        try replacePaperRows(snapshot.journal, table: "paper_journal", db: db)
        try setSetting(snapshot.selectedAccountID, key: PaperKey.selectedAccountID, db: db)
        try setSetting(snapshot.quotes.isEmpty ? nil : snapshot.quotes, key: PaperKey.quotes, db: db)
    }

    /// A row that no longer decodes fails the read: dropping a fill or order would
    /// silently change the account's cash and positions.
    private static func paperRows<T: Decodable>(_ type: T.Type, table: String, db: Database) throws -> [T] {
        try String.fetchAll(db, sql: "SELECT payload FROM \(table) ORDER BY rowid")
            .map { try decoder.decode(T.self, from: Data($0.utf8)) }
    }

    private static func replacePaperRows<T: Encodable & PaperAccountScoped>(
        _ items: [T], table: String, db: Database
    ) throws {
        try db.execute(sql: "DELETE FROM \(table)")
        let statement = try db.makeStatement(sql: "INSERT INTO \(table) (account_id, payload) VALUES (?, ?)")
        for item in items {
            try statement.execute(arguments: [item.accountID.uuidString, try json(item)])
        }
    }
}

protocol PaperAccountScoped {
    var accountID: UUID { get }
}

extension PaperOrder: PaperAccountScoped {}
extension PaperFill: PaperAccountScoped {}
extension PaperPosition: PaperAccountScoped {}
extension PaperOrderEvent: PaperAccountScoped {}
extension PaperClosedTrade: PaperAccountScoped {}
extension PaperJournalEntry: PaperAccountScoped {}
