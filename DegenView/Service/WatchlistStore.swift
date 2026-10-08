import Foundation
import GRDB

/// Every watchlist and colour flag, shared by all tabs and windows. The single source of
/// truth for what used to be Favorites: the chart-card star is membership of the list
/// marked `isFavorites`.
///
/// Mutations validate, write one transaction, and only then publish, so memory and disk
/// agree after a failed write. If the saved lists could not be read, `loadFailed` is set
/// and every mutation is refused rather than writing over them.
///
/// Live prices never come through here: quotes are transient (`WatchlistQuoteBook`) and
/// nothing in this store is written on a tick.
@MainActor
final class WatchlistStore: ObservableObject {
    static let shared = WatchlistStore()

    @Published private(set) var lists: [Watchlist] = []
    /// Colour marks by `InstrumentID.key`, shared by every list.
    @Published private(set) var flags: [String: WatchlistFlag] = [:]
    @Published private(set) var loadFailed = false

    let database: AppDatabase
    private let now: () -> Date

    static let flagsKey = "watchlist.flags"
    static let lastSelectedKey = "watchlist.lastSelected"
    static let migratedKey = "watchlist.favoritesMigrated"

    init(database: AppDatabase = .shared, now: @escaping () -> Date = Date.init) {
        self.database = database
        self.now = now
        do {
            let loaded = try database.writer.write { db -> (lists: [Watchlist], flags: [String: WatchlistFlag]) in
                var lists = try AppDatabase.documentsStrict(Watchlist.self, in: .watchlist, db: db)
                if lists.isEmpty, try AppDatabase.setting(Bool.self, key: Self.migratedKey, db: db) != true {
                    lists = [try Self.migratedFavorites(db: db, now: now())]
                    try AppDatabase.setSetting(true, key: Self.migratedKey, db: db)
                    try AppDatabase.upsertDocument(lists[0], position: 0, in: .watchlist, db: db)
                } else if !lists.contains(where: \.isFavorites) {
                    let favorites = Watchlist(name: "Favorites", isFavorites: true, createdAt: now())
                    lists.insert(favorites, at: 0)
                    for (position, list) in lists.enumerated() {
                        try AppDatabase.upsertDocument(list, position: position, in: .watchlist, db: db)
                    }
                }
                let flags =
                    try AppDatabase.setting([String: WatchlistFlag].self, key: Self.flagsKey, db: db) ?? [:]
                return (lists, flags)
            }
            lists = loaded.lists
            flags = loaded.flags
        } catch {
            #if DEBUG
                print("[WatchlistStore] Could not load watchlists: \(error)")
            #endif
            loadFailed = true
        }
    }

    // MARK: Reading

    func list(_ id: UUID) -> Watchlist? {
        lists.first { $0.id == id }
    }

    var favorites: Watchlist? {
        lists.first(where: \.isFavorites)
    }

    /// Lists that hold this market, in sidebar order.
    func lists(containing instrument: InstrumentID) -> [Watchlist] {
        lists.filter { $0.contains(instrument) }
    }

    func isFavorite(_ instrument: InstrumentID) -> Bool {
        favorites?.contains(instrument) ?? false
    }

    func flag(for instrument: InstrumentID) -> WatchlistFlag? {
        flags[instrument.key]
    }

    /// The list a new window shows first. Per-window selection lives in the view model.
    var lastSelectedID: UUID? {
        get { database.setting(UUID.self, key: Self.lastSelectedKey) }
        set { database.setSetting(newValue, key: Self.lastSelectedKey) }
    }

    /// `preferred` if it still exists, otherwise the favorites list, otherwise any list.
    func resolvedSelection(_ preferred: UUID?) -> UUID? {
        if let preferred, list(preferred) != nil { return preferred }
        if let last = lastSelectedID, list(last) != nil { return last }
        return (favorites ?? lists.first)?.id
    }

    // MARK: Lists

    @discardableResult
    func createWatchlist(name: String) throws -> Watchlist {
        let list = Watchlist(name: try Watchlist.validated(name), createdAt: now())
        try commit(lists + [list])
        return list
    }

    func renameWatchlist(id: UUID, name: String) throws {
        let clean = try Watchlist.validated(name)
        try mutate(id) { $0.name = clean }
    }

    @discardableResult
    func duplicateWatchlist(id: UUID, name: String? = nil) throws -> Watchlist {
        try requireWritable()
        guard let source = list(id) else { throw WatchlistError.notFound }
        let copy = source.duplicated(
            name: try Watchlist.validated(name ?? "\(source.name) copy"), at: now())
        var updated = lists
        updated.insert(copy, at: (updated.firstIndex { $0.id == id } ?? updated.count - 1) + 1)
        try commit(updated)
        return copy
    }

    func deleteWatchlist(id: UUID) throws {
        try requireWritable()
        guard let target = list(id) else { throw WatchlistError.notFound }
        guard !target.isFavorites else { throw WatchlistError.lastFavorites }
        try commit(lists.filter { $0.id != id })
    }

    /// Moves a list before another, or to the end when `targetID` is nil.
    func moveWatchlist(_ id: UUID, before targetID: UUID?) throws {
        try requireWritable()
        guard let from = lists.firstIndex(where: { $0.id == id }) else { throw WatchlistError.notFound }
        var updated = lists
        let moving = updated.remove(at: from)
        let destination = targetID.flatMap { target in updated.firstIndex { $0.id == target } } ?? updated.count
        updated.insert(moving, at: destination)
        try commit(updated)
    }

    // MARK: Instruments

    func addInstrument(_ instrument: WatchlistInstrument, to listID: UUID, section: UUID? = nil) throws {
        try mutate(listID) { try $0.add(instrument, toSection: section) }
    }

    func add(_ result: TickerSearchResult, to listID: UUID, section: UUID? = nil) throws {
        try addInstrument(WatchlistInstrument(searchResult: result), to: listID, section: section)
    }

    func removeInstrument(_ instrument: InstrumentID, from listID: UUID) throws {
        try mutate(listID) { list in
            guard list.removeInstrument(instrument) else { throw WatchlistError.notFound }
        }
    }

    func removeEntry(_ entryID: UUID, from listID: UUID) throws {
        try mutate(listID) { try $0.removeEntry(entryID) }
    }

    /// Star on a chart card: add to or remove from the Favorites list.
    func toggleFavorite(_ instrument: WatchlistInstrument) throws {
        try requireWritable()
        guard let favorites else { throw WatchlistError.notFound }
        if favorites.contains(instrument.instrument) {
            try removeInstrument(instrument.instrument, from: favorites.id)
        } else {
            try addInstrument(instrument, to: favorites.id)
        }
    }

    func moveEntry(_ entryID: UUID, before targetID: UUID?, in listID: UUID) throws {
        try mutate(listID) { try $0.move(entryID, before: targetID) }
    }

    /// One drop, one transaction: every moved entry lands before `targetID` (nil = the end), in the order given.
    func moveEntries(_ entryIDs: [UUID], before targetID: UUID?, in listID: UUID) throws {
        try mutate(listID) { try $0.move(entryIDs, before: targetID) }
    }

    func moveInstrument(_ entryID: UUID, toSection sectionID: UUID?, in listID: UUID) throws {
        try mutate(listID) { try $0.moveInstrument(entryID, toSection: sectionID) }
    }

    /// Records a DEX pair's network once it is known. Every list holding the pair gets it.
    func setChain(_ chain: String, for instrument: InstrumentID) throws {
        try requireWritable()
        var updated = lists
        var changed = false
        for index in updated.indices where updated[index].entry(for: instrument)?.instrument.chain != chain {
            guard updated[index].contains(instrument) else { continue }
            updated[index].setChain(chain, for: instrument)
            changed = true
        }
        if changed { try commit(updated) }
    }

    // MARK: Sections

    @discardableResult
    func addSection(title: String, in listID: UUID, before targetID: UUID? = nil) throws -> WatchlistSection {
        var created: WatchlistSection?
        try mutate(listID) { created = try $0.addSection(title: title, before: targetID) }
        guard let created else { throw WatchlistError.notFound }
        return created
    }

    func renameSection(_ id: UUID, to title: String, in listID: UUID) throws {
        try mutate(listID) { try $0.renameSection(id, to: title) }
    }

    func deleteSection(_ id: UUID, in listID: UUID) throws {
        try mutate(listID) { try $0.deleteSection(id) }
    }

    func setSectionCollapsed(_ id: UUID, _ collapsed: Bool, in listID: UUID) throws {
        try mutate(listID) { try $0.setSectionCollapsed(id, collapsed) }
    }

    // MARK: Display and flags

    func updateDisplay(of listID: UUID, _ change: (inout WatchlistDisplaySettings) -> Void) throws {
        try mutate(listID) { change(&$0.display) }
    }

    func setFlag(_ flag: WatchlistFlag?, for instrument: InstrumentID) throws {
        try requireWritable()
        var updated = flags
        updated[instrument.key] = flag
        guard updated != flags else { return }
        do {
            try database.writer.write { try AppDatabase.setSetting(updated, key: Self.flagsKey, db: $0) }
        } catch {
            throw WatchlistError.writeFailed(error.localizedDescription)
        }
        flags = updated
    }

    // MARK: Text

    func exportText(of listID: UUID) -> String? {
        list(listID).map(WatchlistTextFormat.export)
    }

    /// Adds every valid row of `text` to a list. Sections are reused by title, instruments
    /// already present are counted, bad rows are reported and never block good ones.
    func importText(_ text: String, into listID: UUID) throws -> WatchlistImportReport {
        let parsed = WatchlistTextFormat.parse(text)
        var report = WatchlistImportReport(skipped: parsed.skipped)
        try mutate(listID) { list in
            var currentSection: UUID?
            for row in parsed.rows {
                switch row {
                case .section(let title):
                    if let existing = list.sections.first(where: {
                        $0.title.caseInsensitiveCompare(title) == .orderedSame
                    }) {
                        currentSection = existing.id
                    } else {
                        currentSection = try list.addSection(title: title).id
                        report.sectionsAdded += 1
                    }
                case .instrument(let item):
                    if list.contains(item.instrument) {
                        report.duplicates += 1
                    } else {
                        try list.add(item, toSection: currentSection)
                        report.added += 1
                    }
                }
            }
        }
        return report
    }

    // MARK: Persistence

    private func requireWritable() throws {
        guard !loadFailed else { throw WatchlistError.persistenceUnavailable }
    }

    private func mutate(_ listID: UUID, _ body: (inout Watchlist) throws -> Void) throws {
        try requireWritable()
        guard let index = lists.firstIndex(where: { $0.id == listID }) else { throw WatchlistError.notFound }
        var updated = lists
        try body(&updated[index])
        guard updated[index] != lists[index] else { return }
        updated[index].updatedAt = now()
        try commit(updated)
    }

    /// Writes only the rows that changed or moved, in one transaction, then publishes.
    private func commit(_ updated: [Watchlist]) throws {
        try requireWritable()
        let old = Dictionary(uniqueKeysWithValues: lists.enumerated().map { ($1.id, ($0, $1)) })
        do {
            try database.writer.write { db in
                let kept = Set(updated.map(\.id))
                for removed in old.keys where !kept.contains(removed) {
                    try AppDatabase.deleteDocument(id: removed, in: .watchlist, db: db)
                }
                for (position, list) in updated.enumerated() {
                    if let previous = old[list.id], previous.0 == position, previous.1 == list { continue }
                    try AppDatabase.upsertDocument(list, position: position, in: .watchlist, db: db)
                }
            }
        } catch {
            throw WatchlistError.writeFailed(error.localizedDescription)
        }
        lists = updated
    }
}
