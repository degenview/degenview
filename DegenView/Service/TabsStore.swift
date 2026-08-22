import Foundation
import Combine

/// Single source of truth for every tab's chart state.
///
/// Replaces the app-global `tickers.json` + `lastViewID` pair: those described
/// one implicit tab, this describes N explicit ones. Saved views (`views.json`)
/// stay global — a shared library any tab can load from.
@MainActor
final class TabsStore: ObservableObject {
    static let shared = TabsStore()

    /// Ordered for deterministic restore; `windowGroups` decides the actual layout.
    @Published private(set) var tabs: [ChartTab] = []

    /// Ordered tab ids per window, captured from AppKit at save time.
    /// Empty until the first `captureGrouping()`.
    private(set) var windowGroups: [[UUID]] = []

    private let store = JSONStore<TabsSnapshot>(filename: "tabs.json")
    private var saveTask: Task<Void, Never>?

    private init() {
        if let snapshot = store.load() {
            tabs = snapshot.tabs
            windowGroups = snapshot.windowGroups
        } else {
            let migrated = migrateFromLegacyStorage()
            tabs = migrated.tabs
            windowGroups = migrated.windowGroups
            // Nothing has mutated yet, so no debounced write is pending — land
            // the migration on disk now rather than on the next edit.
            store.save(migrated)
        }
    }

    // MARK: - Lookup

    func tab(_ id: UUID) -> ChartTab? {
        tabs.first { $0.id == id }
    }

    /// The tab the first window should adopt when SwiftUI hands it a nil value.
    /// Falls back to minting one so a cold install still opens a usable window.
    func firstTabID() -> UUID {
        if let first = windowGroups.first?.first, tab(first) != nil { return first }
        if let first = tabs.first { return first.id }
        return makeTab().id
    }

    /// Every persisted tab except `firstTabID()`, in the order windows should
    /// reopen them.
    func remainingTabIDs(excluding adopted: UUID) -> [UUID] {
        let grouped = windowGroups.flatMap { $0 }.filter { tab($0) != nil }
        let ordered = grouped.isEmpty ? tabs.map(\.id) : grouped
        var seen = Set([adopted])
        return ordered.filter { seen.insert($0).inserted }
    }

    /// Which window group a tab belonged to at quit, as an index into
    /// `windowGroups`. Nil when the tab predates any capture.
    func windowIndex(of id: UUID) -> Int? {
        windowGroups.firstIndex { $0.contains(id) }
    }

    // MARK: - Mutation

    @discardableResult
    func makeTab() -> ChartTab {
        let tab = ChartTab()
        tabs.append(tab)
        scheduleSave()
        return tab
    }

    /// Create a tab that is ready to render a selected shortcut on its first frame.
    @discardableResult
    func makeTab(name: String, tickerConfig: TickerConfig) -> ChartTab {
        let tab = ChartTab(name: name, tickerConfigs: [tickerConfig])
        tabs.append(tab)
        scheduleSave()
        return tab
    }

    /// Fetch-or-create, so a window handed an unknown id still opens.
    func ensureTab(_ id: UUID) -> ChartTab {
        if let existing = tab(id) { return existing }
        let tab = ChartTab(id: id)
        tabs.append(tab)
        scheduleSave()
        return tab
    }

    func update(_ id: UUID, _ mutate: (inout ChartTab) -> Void) {
        guard let index = tabs.firstIndex(where: { $0.id == id }) else { return }
        mutate(&tabs[index])
        scheduleSave()
    }

    func removeTab(_ id: UUID) {
        tabs.removeAll { $0.id == id }
        windowGroups = windowGroups
            .map { $0.filter { $0 != id } }
            .filter { !$0.isEmpty }
        scheduleSave()
    }

    func setWindowGroups(_ groups: [[UUID]]) {
        windowGroups = groups
        scheduleSave()
    }

    // MARK: - Persistence

    /// Coalesce the writes that a drag-zoom or a burst of ticker edits produces.
    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: CacheLimit.saveDebounceNS)
            guard !Task.isCancelled else { return }
            self?.persist()
        }
    }

    /// Write immediately. Called on quit, where the debounce would never fire.
    func persist() {
        saveTask?.cancel()
        saveTask = nil
        store.save(TabsSnapshot(tabs: tabs, windowGroups: windowGroups))
    }

    // MARK: - Migration

    /// Fold the pre-tabs single-document state into one tab.
    ///
    /// Mirrors what launch used to do: `lastViewID` won over `tickers.json`,
    /// because restoring a saved view replaced the ticker list wholesale.
    private func migrateFromLegacyStorage() -> TabsSnapshot {
        let savedViews = JSONStore<[SavedView]>(filename: "views.json").load() ?? []

        if let idString = UserDefaults.standard.string(forKey: "lastViewID"),
           let id = UUID(uuidString: idString),
           let view = savedViews.first(where: { $0.id == id }) {
            let tab = ChartTab(
                name: view.name,
                savedViewID: view.id,
                tickerConfigs: view.resolvedConfigs,
                timeRange: view.timeRange,
                layoutMode: view.layoutMode,
                candleCount: view.candleCount ?? view.timeRange.dataPointLimit
            )
            return TabsSnapshot(tabs: [tab], windowGroups: [[tab.id]])
        }

        let configs = JSONStore<[TickerConfig]>(filename: "tickers.json").load()
            ?? legacyStringTickers()
        let tab = ChartTab(tickerConfigs: configs)
        return TabsSnapshot(tabs: [tab], windowGroups: [[tab.id]])
    }

    /// The oldest on-disk format: a bare `[String]` of Binance symbols.
    private func legacyStringTickers() -> [TickerConfig] {
        guard let data = try? Data(contentsOf: AppSupport.directory.appendingPathComponent("tickers.json")),
              let strings = try? JSONDecoder().decode([String].self, from: data)
        else { return [] }
#if DEBUG
        print("[TabsStore] Migrated \(strings.count) legacy tickers to .binance")
#endif
        return strings.map { TickerConfig(symbol: $0, source: .binance) }
    }
}
