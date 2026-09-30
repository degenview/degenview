import Foundation
import GRDB

/// One stored candle. Plain values so this layer doesn't depend on `KlineData`.
struct CandleRecord: Equatable, Sendable {
    var openTime: Double
    var open: Double
    var high: Double
    var low: Double
    var close: Double
    var volume: Double
    var quoteVolume: Double
}

/// Which stretch of a series has been fetched. Every candle the source has from
/// `coveredFrom` (an instant, not a candle) to `newest` (a candle open time) is stored.
/// `coveredFrom` can sit before the first candle: the source was asked and had nothing older.
struct CandleCoverage: Equatable, Sendable {
    var coveredFrom: Double
    var newest: Double
}

extension AppDatabase {
    func candles(source: String, symbol: String, interval: String, from: Double) -> [CandleRecord] {
        do {
            return try reader.read { db in
                try Row.fetchAll(
                    db,
                    sql: """
                        SELECT open_time, open, high, low, close, volume, quote_volume FROM candle
                        WHERE source = ? AND symbol = ? AND interval = ? AND open_time >= ?
                        ORDER BY open_time
                        """,
                    arguments: [source, symbol, interval, from]
                ).map {
                    CandleRecord(
                        openTime: $0["open_time"], open: $0["open"], high: $0["high"], low: $0["low"],
                        close: $0["close"], volume: $0["volume"], quoteVolume: $0["quote_volume"])
                }
            }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not read candles: \(error.localizedDescription)")
            #endif
            return []
        }
    }

    func candleCoverage(source: String, symbol: String, interval: String) -> CandleCoverage? {
        try? reader.read { db in
            try Row.fetchOne(
                db,
                sql: """
                    SELECT covered_from, newest FROM candle_coverage
                    WHERE source = ? AND symbol = ? AND interval = ?
                    """,
                arguments: [source, symbol, interval]
            ).map { CandleCoverage(coveredFrom: $0["covered_from"], newest: $0["newest"]) }
        }
    }

    /// Stores the candles and the widened coverage in one transaction, so coverage never
    /// claims a range whose candles didn't land.
    func storeCandles(
        _ candles: [CandleRecord], coverage: CandleCoverage, source: String, symbol: String, interval: String
    ) {
        do {
            try writer.write { db in
                let statement = try db.makeStatement(
                    sql: """
                        INSERT OR REPLACE INTO candle
                        (source, symbol, interval, open_time, open, high, low, close, volume, quote_volume)
                        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                        """)
                for candle in candles {
                    try statement.execute(arguments: [
                        source, symbol, interval, candle.openTime, candle.open, candle.high, candle.low,
                        candle.close, candle.volume, candle.quoteVolume,
                    ])
                }
                try db.execute(
                    sql: """
                        INSERT OR REPLACE INTO candle_coverage (source, symbol, interval, covered_from, newest)
                        VALUES (?, ?, ?, ?, ?)
                        """,
                    arguments: [source, symbol, interval, coverage.coveredFrom, coverage.newest])
            }
        } catch {
            #if DEBUG
                print("[AppDatabase] Could not write candles: \(error.localizedDescription)")
            #endif
        }
    }
}
