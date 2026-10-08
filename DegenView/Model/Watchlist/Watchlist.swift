import Foundation

/// A named, ordered list of markets and section dividers.
///
/// `entries` is one flat array. A section owns the instruments after it until the next
/// section, so reordering anything is a single move and deleting a section only removes
/// its header: its instruments fall into the section above, or the root.
///
/// The mutating methods keep the invariants (unique entry ids, one entry per market) and
/// know nothing about persistence; `WatchlistStore` wraps them.
struct Watchlist: Identifiable, Codable, Equatable, Sendable {
    let id: UUID
    var name: String
    var entries: [WatchlistEntry]
    var display: WatchlistDisplaySettings
    /// The list the chart-card star adds to. Exactly one list has this; it can be renamed
    /// but not deleted.
    var isFavorites: Bool
    var createdAt: Date
    var updatedAt: Date

    init(
        id: UUID = UUID(), name: String, entries: [WatchlistEntry] = [],
        display: WatchlistDisplaySettings = WatchlistDisplaySettings(), isFavorites: Bool = false,
        createdAt: Date = Date(), updatedAt: Date? = nil
    ) {
        self.id = id
        self.name = name
        self.entries = entries
        self.display = display
        self.isFavorites = isFavorites
        self.createdAt = createdAt
        self.updatedAt = updatedAt ?? createdAt
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(UUID.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        entries = try container.decodeIfPresent([WatchlistEntry].self, forKey: .entries) ?? []
        display = try container.decodeIfPresent(WatchlistDisplaySettings.self, forKey: .display) ?? .init()
        isFavorites = try container.decodeIfPresent(Bool.self, forKey: .isFavorites) ?? false
        createdAt = try container.decodeIfPresent(Date.self, forKey: .createdAt) ?? Date()
        updatedAt = try container.decodeIfPresent(Date.self, forKey: .updatedAt) ?? createdAt
    }

    // MARK: Reading

    var instruments: [WatchlistInstrument] { entries.compactMap(\.instrument) }
    var sections: [WatchlistSection] { entries.compactMap(\.section) }

    func contains(_ instrument: InstrumentID) -> Bool {
        entry(for: instrument) != nil
    }

    func entry(for instrument: InstrumentID) -> WatchlistInstrument? {
        instruments.first { $0.instrument == instrument }
    }

    func index(of entryID: UUID) -> Int? {
        entries.firstIndex { $0.id == entryID }
    }

    /// The section an entry sits under, or nil at the root.
    func owningSection(of entryID: UUID) -> WatchlistSection? {
        guard let index = index(of: entryID) else { return nil }
        for candidate in entries[..<index].reversed() {
            if let section = candidate.section { return section }
        }
        return nil
    }

    /// Entries a collapsed section hides, for subscription decisions.
    var visibleInstruments: [WatchlistInstrument] {
        var result: [WatchlistInstrument] = []
        var collapsed = false
        for entry in entries {
            switch entry {
            case .section(let section): collapsed = section.isCollapsed
            case .instrument(let instrument): if !collapsed { result.append(instrument) }
            }
        }
        return result
    }

    // MARK: Instruments

    /// Adds at the end of a section's block, or at the end of the list when `sectionID` is nil.
    mutating func add(_ instrument: WatchlistInstrument, toSection sectionID: UUID? = nil) throws {
        guard !contains(instrument.instrument) else { throw WatchlistError.duplicate(instrument.name) }
        guard !entries.contains(where: { $0.id == instrument.id }) else { throw WatchlistError.duplicate(instrument.name) }
        let index: Int
        if let sectionID {
            guard let sectionIndex = self.index(of: sectionID), entries[sectionIndex].section != nil else {
                throw WatchlistError.notFound
            }
            index = blockEnd(afterSectionAt: sectionIndex)
        } else {
            index = entries.count
        }
        entries.insert(.instrument(instrument), at: index)
    }

    @discardableResult
    mutating func removeInstrument(_ instrument: InstrumentID) -> Bool {
        guard let index = entries.firstIndex(where: { $0.instrument?.instrument == instrument }) else { return false }
        entries.remove(at: index)
        return true
    }

    mutating func removeEntry(_ entryID: UUID) throws {
        guard let index = index(of: entryID), entries[index].instrument != nil else { throw WatchlistError.notFound }
        entries.remove(at: index)
    }

    /// Replaces the stored DEX chain, which older favorites never kept.
    mutating func setChain(_ chain: String, for instrument: InstrumentID) {
        for index in entries.indices {
            guard var item = entries[index].instrument, item.instrument == instrument else { continue }
            item.instrument.chain = chain
            entries[index] = .instrument(item)
        }
    }

    // MARK: Ordering

    /// Moves an instrument or a whole section (header plus its block) to sit before
    /// `targetID`, or to the end when nil. Dropping onto itself, or a section into its own
    /// block, changes nothing.
    mutating func move(_ entryID: UUID, before targetID: UUID?) throws {
        guard let from = index(of: entryID) else { throw WatchlistError.notFound }
        if let targetID, index(of: targetID) == nil { throw WatchlistError.notFound }
        if targetID == entryID { return }

        if entries[from].section != nil {
            let range = from..<blockEnd(afterSectionAt: from)
            var destination = targetID.flatMap(index(of:)).map(blockStart(containing:)) ?? entries.count
            if range.contains(destination) || destination == range.upperBound { return }
            let block = Array(entries[range])
            entries.removeSubrange(range)
            if destination > range.lowerBound { destination -= block.count }
            entries.insert(contentsOf: block, at: destination)
        } else {
            let entry = entries.remove(at: from)
            let destination = targetID.flatMap(index(of:)) ?? entries.count
            entries.insert(entry, at: destination)
        }
    }

    /// Moves several entries together to sit before `targetID` (or at the end), keeping their relative order.
    /// Nothing moves if any id is unknown.
    mutating func move(_ ids: [UUID], before targetID: UUID?) throws {
        guard ids.allSatisfy({ index(of: $0) != nil }) else { throw WatchlistError.notFound }
        if let targetID, ids.contains(targetID) { return }
        for id in ids { try move(id, before: targetID) }
    }

    /// Moves an instrument to the end of a section's block, or to the end of the root block.
    mutating func moveInstrument(_ entryID: UUID, toSection sectionID: UUID?) throws {
        guard let from = index(of: entryID), entries[from].instrument != nil else { throw WatchlistError.notFound }
        let entry = entries.remove(at: from)
        let destination: Int
        if let sectionID {
            guard let sectionIndex = index(of: sectionID), entries[sectionIndex].section != nil else {
                entries.insert(entry, at: from)
                throw WatchlistError.notFound
            }
            destination = blockEnd(afterSectionAt: sectionIndex)
        } else {
            destination = entries.firstIndex { $0.section != nil } ?? entries.count
        }
        entries.insert(entry, at: destination)
    }

    // MARK: Sections

    @discardableResult
    mutating func addSection(title: String, before targetID: UUID? = nil) throws -> WatchlistSection {
        let section = WatchlistSection(title: try Self.validated(title))
        let destination = targetID.flatMap(index(of:)) ?? entries.count
        entries.insert(.section(section), at: destination)
        return section
    }

    mutating func renameSection(_ id: UUID, to title: String) throws {
        let clean = try Self.validated(title)
        guard let index = index(of: id), var section = entries[index].section else { throw WatchlistError.notFound }
        section.title = clean
        entries[index] = .section(section)
    }

    mutating func setSectionCollapsed(_ id: UUID, _ collapsed: Bool) throws {
        guard let index = index(of: id), var section = entries[index].section else { throw WatchlistError.notFound }
        section.isCollapsed = collapsed
        entries[index] = .section(section)
    }

    /// Removes the header only; its instruments join the section above, or the root.
    mutating func deleteSection(_ id: UUID) throws {
        guard let index = index(of: id), entries[index].section != nil else { throw WatchlistError.notFound }
        entries.remove(at: index)
    }

    // MARK: Whole list

    /// A copy under new identities throughout, so it never shares a row id with the original.
    func duplicated(name: String, at now: Date = Date()) -> Watchlist {
        Watchlist(
            name: name,
            entries: entries.map { entry in
                switch entry {
                case .instrument(let item): return .instrument(item.copy())
                case .section(let section):
                    return .section(WatchlistSection(title: section.title, isCollapsed: section.isCollapsed))
                }
            },
            display: display, isFavorites: false, createdAt: now)
    }

    /// Reasons this list breaks an invariant. Empty for a healthy list.
    func validate() -> [String] {
        var problems: [String] = []
        var ids = Set<UUID>()
        var markets = Set<InstrumentID>()
        for entry in entries {
            if !ids.insert(entry.id).inserted { problems.append("duplicate entry id \(entry.id)") }
            if let item = entry.instrument, !markets.insert(item.instrument).inserted {
                problems.append("duplicate market \(item.instrument.key)")
            }
        }
        return problems
    }

    static func validated(_ name: String) throws -> String {
        let clean = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { throw WatchlistError.emptyName }
        return clean
    }

    // MARK: Block geometry

    /// Index just past the last instrument that belongs to the section at `sectionIndex`.
    private func blockEnd(afterSectionAt sectionIndex: Int) -> Int {
        var index = sectionIndex + 1
        while index < entries.count, entries[index].section == nil { index += 1 }
        return index
    }

    /// Where a section dropped "before" this entry should land: the header of the block the
    /// entry sits in, so a block is never split.
    private func blockStart(containing index: Int) -> Int {
        var cursor = index
        while cursor > 0, entries[cursor].section == nil { cursor -= 1 }
        return entries[cursor].section == nil ? 0 : cursor
    }
}
