import Foundation

/// The one in-process owner of the saved-layout library.
///
/// Every tab reads and writes through here, so a tab that has been open for a while can no longer
/// overwrite a layout another tab saved: the table is replaced from this list, which is always current.
/// Every mutation persists first and only then updates `views`, so a failed write leaves memory and
/// disk agreeing on the old state.
@MainActor
final class SavedViewStore: ObservableObject {
    static let shared = SavedViewStore(database: .shared)

    @Published private(set) var views: [SavedView]

    private let load: () -> [SavedView]
    private let persist: ([SavedView]) throws -> Void
    private let now: () -> Date

    init(
        database: AppDatabase,
        persist: (([SavedView]) throws -> Void)? = nil,
        now: @escaping () -> Date = Date.init
    ) {
        self.load = { database.savedViews() }
        self.persist = persist ?? { try database.writeSavedViews($0) }
        self.now = now
        self.views = database.savedViews()
    }

    func view(id: UUID?) -> SavedView? {
        guard let id else { return nil }
        return views.first { $0.id == id }
    }

    /// Re-read the table, for changes made by something other than this store.
    func reload() {
        views = load()
    }

    /// Layouts opened or saved before, newest first. Never-opened layouts are left out until opened.
    func recent(excluding id: UUID? = nil, limit: Int = SavedLayout.recentLimit) -> [SavedView] {
        let opened = views.compactMap { view -> (SavedView, Date)? in
            guard view.id != id, let date = view.lastOpenedAt else { return nil }
            return (view, date)
        }
        return opened.sorted { $0.1 > $1.1 }.prefix(limit).map(\.0)
    }

    /// Insert, or replace in place so the layout keeps its position. Stamps recency.
    func upsert(_ view: SavedView) throws {
        var stamped = view
        stamped.lastOpenedAt = now()
        var next = views
        if let index = next.firstIndex(where: { $0.id == view.id }) {
            next[index] = stamped
        } else {
            next.append(stamped)
        }
        try commit(next)
    }

    /// Identity, content, position and recency are untouched.
    func rename(id: UUID, to name: String) throws {
        try update(id) { $0.name = name }
    }

    func markOpened(id: UUID) throws {
        try update(id) { $0.lastOpenedAt = now() }
    }

    func setAutosave(id: UUID, _ enabled: Bool) throws {
        try update(id) { $0.autosave = enabled ? true : nil }
    }

    func delete(id: UUID) throws {
        try commit(views.filter { $0.id != id })
    }

    private func update(_ id: UUID, _ change: (inout SavedView) -> Void) throws {
        guard let index = views.firstIndex(where: { $0.id == id }) else { return }
        var next = views
        change(&next[index])
        try commit(next)
    }

    private func commit(_ next: [SavedView]) throws {
        try persist(next)
        views = next
    }
}
