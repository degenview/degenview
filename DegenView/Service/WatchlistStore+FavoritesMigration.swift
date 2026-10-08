import Foundation
import GRDB

extension WatchlistStore {
    /// The old `favorite` row shape. Those rows stay in the table untouched; this is the
    /// only reader, and it runs once.
    private struct LegacyFavorite: Decodable {
        let id: UUID
        let name: String
        let ticker: String
        let config: TickerConfig
    }

    /// Builds the default "Favorites" list from the legacy table, keeping order and each
    /// favorite's id. Rows that no longer decode are skipped, as the old store skipped them.
    static func migratedFavorites(db: Database, now: Date) throws -> Watchlist {
        let legacy = try AppDatabase.documents(LegacyFavorite.self, in: .favorite, db: db)
        var seen = Set<InstrumentID>()
        var entries: [WatchlistEntry] = []
        for favorite in legacy {
            let instrument = InstrumentID(source: favorite.config.source, symbol: favorite.config.symbol)
            guard instrument.source.isWatchlistInstrumentSource, seen.insert(instrument).inserted else { continue }
            entries.append(
                .instrument(
                    WatchlistInstrument(
                        id: favorite.id, instrument: instrument, name: favorite.name, label: favorite.ticker,
                        displayName: favorite.config.displayName, pmSeries: favorite.config.pmSeries)))
        }
        return Watchlist(name: "Favorites", entries: entries, isFavorites: true, createdAt: now)
    }
}
