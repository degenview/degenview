import Combine
import Foundation

/// A tab's relationship to the saved-layout library: which layout it is on, whether it has drifted,
/// and every operation the toolbar menu offers.
///
/// Dirty state is never tracked by flags. `refresh()` compares a `LayoutSnapshot` of the live tab
/// with the snapshot taken when the layout was last opened or saved, and `ContentViewModel` calls
/// it from `syncTab()` — which only configuration changes reach, never market-data ticks.
///
/// Autosave is per layout (`SavedView.autosave`) and only exists for a layout that has a name: an
/// Unnamed tab never creates a saved layout on its own, it waits for the user's first Save.
/// Two tabs on the same layout each save independently; the last write wins.
@MainActor
final class SavedLayoutController: ObservableObject {
    /// What the tab looks like right now, as the controller needs it. Supplied by `ContentViewModel`.
    struct Capture {
        var timeRange: TimeRange
        var configs: [TickerConfig]
        var columns: [ChartColumn]
        var candleCount: Int

        var snapshot: LayoutSnapshot {
            LayoutSnapshot(timeRange: timeRange, configs: configs, columns: columns)
        }
    }

    /// A change of layout that must wait for unsaved changes to be resolved.
    enum Transition: Equatable {
        case open(UUID)
    }

    /// A name the user has to type before the operation can finish.
    enum Prompt: Equatable {
        /// First save of an Unnamed tab, optionally followed by the switch that asked for it.
        case save(then: Transition?)
        case copy
        case rename
    }

    enum Choice {
        case save, discard, cancel
    }

    let store: SavedViewStore

    @Published private(set) var activeViewID: UUID?
    @Published private(set) var isDirty = false
    /// The last autosave did not reach disk; the layout is still dirty.
    @Published private(set) var autosaveFailed = false
    @Published private(set) var lastError: String?
    @Published var prompt: Prompt?
    @Published var pendingTransition: Transition?

    /// Supplies the live tab. Set by the owner once it can capture itself.
    var capture: () -> Capture = {
        Capture(timeRange: .oneDay, configs: [], columns: [], candleCount: TimeRange.oneDay.dataPointLimit)
    }
    /// Rebuilds the tab from a saved layout. Runs with dirty tracking suspended.
    var applyView: (SavedView) -> Void = { _ in }
    /// Called whenever the active layout or its name changes.
    var onIdentityChange: () -> Void = {}
    var openNewTab: () -> Void = {}

    /// The in-flight debounced autosave, exposed so tests can await it.
    private(set) var pendingAutosave: Task<Void, Never>?

    private var baseline: LayoutSnapshot
    private var isRestoring = false
    private var generation = 0
    private let sleep: (UInt64) async throws -> Void
    private let autosaveDelay: UInt64
    private var storeObserver: AnyCancellable?

    init(
        store: SavedViewStore,
        activeViewID: UUID?,
        autosaveDelay: UInt64 = SavedLayout.autosaveDebounceNS,
        sleep: @escaping (UInt64) async throws -> Void = { try await Task.sleep(nanoseconds: $0) }
    ) {
        self.store = store
        self.autosaveDelay = autosaveDelay
        self.sleep = sleep
        let active = store.view(id: activeViewID)
        self.activeViewID = active?.id
        self.baseline = active.map { LayoutSnapshot(view: $0) } ?? .empty

        // Another tab may rename, delete or open layouts; keep the toolbar and tab title current.
        storeObserver = store.$views
            .dropFirst()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in
                MainActor.assumeIsolated { self?.storeDidChange() }
            }
    }

    // MARK: - Derived state

    var activeView: SavedView? { store.view(id: activeViewID) }
    var displayName: String { activeView?.name ?? UI.unnamedView }
    var isUnnamed: Bool { activeViewID == nil }
    var autosaveEnabled: Bool { activeView?.autosave == true }
    var recentViews: [SavedView] { store.recent(excluding: activeViewID) }

    /// Autosave keeps a layout clean in the user's eyes, so the "(unsaved)" label only appears when
    /// nothing is going to save it — autosave off, or the last autosave failed.
    var showsSaveAffordance: Bool {
        isDirty && !(autosaveEnabled && !autosaveFailed)
    }

    var promptDefaultName: String {
        switch prompt {
        case .copy: return "\(displayName) copy"
        case .rename: return isUnnamed ? "" : displayName
        case .save, nil: return ""
        }
    }

    func clearError() { lastError = nil }

    // MARK: - Dirty tracking

    /// Re-evaluate dirtiness against the baseline and (re)arm or cancel the autosave.
    func refresh() {
        guard !isRestoring else { return }
        let dirty = capture().snapshot != baseline
        if dirty != isDirty { isDirty = dirty }
        if dirty {
            scheduleAutosave()
        } else {
            cancelAutosave()
        }
    }

    // MARK: - Save

    /// ⌘S and the menu item: save in place, or ask for a name the first time.
    func requestSave() {
        if isUnnamed {
            prompt = .save(then: nil)
        } else {
            saveActive()
        }
    }

    /// `reportErrors` is off for autosave, which only flags the failure on the toolbar instead of
    /// interrupting with an alert every time the debounce fires.
    @discardableResult
    func saveActive(reportErrors: Bool = true) -> Bool {
        guard let existing = activeView else { return false }
        let captured = capture()
        let view = makeView(id: existing.id, name: existing.name, autosave: existing.autosave, from: captured)
        return commit(view, from: captured, failure: "save “\(existing.name)”", reportErrors: reportErrors)
    }

    /// First save of an Unnamed tab: mints the layout's identity.
    @discardableResult
    func save(named name: String) -> Bool {
        let captured = capture()
        let view = makeView(id: UUID(), name: name, autosave: nil, from: captured)
        return commit(view, from: captured, failure: "save the layout")
    }

    /// Duplicates what the tab shows right now, unsaved changes included, under a new identity.
    @discardableResult
    func makeCopy(named name: String) -> Bool {
        guard let existing = activeView else { return false }
        let captured = capture()
        let view = makeView(id: UUID(), name: name, autosave: existing.autosave, from: captured)
        return commit(view, from: captured, failure: "copy “\(existing.name)”")
    }

    /// Renames in place. An Unnamed tab has nothing to rename, so it becomes its first save.
    @discardableResult
    func rename(to name: String) -> Bool {
        guard let id = activeViewID else { return save(named: name) }
        do {
            try store.rename(id: id, to: name)
        } catch {
            lastError = "Couldn’t rename the layout: \(error.localizedDescription)"
            return false
        }
        onIdentityChange()
        return true
    }

    func setAutosave(_ enabled: Bool) {
        guard let id = activeViewID else { return }
        do {
            try store.setAutosave(id: id, enabled)
        } catch {
            lastError = "Couldn’t change Autosave: \(error.localizedDescription)"
            return
        }
        autosaveFailed = false
        refresh()
    }

    func delete(_ id: UUID) {
        do {
            try store.delete(id: id)
        } catch {
            lastError = "Couldn’t delete the layout: \(error.localizedDescription)"
            return
        }
        if id == activeViewID { detach() }
    }

    /// Write a pending autosave now, before the tab goes away or switches layout.
    /// False means the layout is still dirty on disk.
    @discardableResult
    func flush() -> Bool {
        cancelAutosave()
        guard autosaveEnabled, isDirty else { return true }
        if saveActive() { return true }
        autosaveFailed = true
        return false
    }

    // MARK: - Switching

    /// Open `view` in this tab, first resolving unsaved changes.
    func requestOpen(_ view: SavedView) {
        let transition = Transition.open(view.id)
        if autosaveEnabled {
            guard flush() else { return }
            perform(transition)
        } else if isDirty {
            pendingTransition = transition
        } else {
            perform(transition)
        }
    }

    func createNewLayout() {
        openNewTab()
    }

    func resolve(_ choice: Choice) {
        guard let transition = pendingTransition else { return }
        pendingTransition = nil
        switch choice {
        case .cancel:
            break
        case .discard:
            perform(transition)
        case .save:
            if isUnnamed {
                prompt = .save(then: transition)
            } else if saveActive() {
                perform(transition)
            }
        }
    }

    func confirmPrompt(name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, let current = prompt else { return }
        prompt = nil
        switch current {
        case .save(let then):
            if save(named: trimmed), let then { perform(then) }
        case .copy:
            makeCopy(named: trimmed)
        case .rename:
            rename(to: trimmed)
        }
    }

    func cancelPrompt() { prompt = nil }

    // MARK: - Internals

    private func perform(_ transition: Transition) {
        switch transition {
        case .open(let id):
            guard let view = store.view(id: id) else { return }
            restore(view)
        }
    }

    /// The one place a saved layout is applied: dirty tracking is off while the tab is rebuilt, and
    /// the baseline is taken from the rebuilt tab so restoring can never leave it looking dirty.
    private func restore(_ view: SavedView) {
        cancelAutosave()
        isRestoring = true
        applyView(view)
        isRestoring = false
        activeViewID = view.id
        baseline = capture().snapshot
        isDirty = false
        autosaveFailed = false
        try? store.markOpened(id: view.id)
        onIdentityChange()
    }

    private func makeView(id: UUID, name: String, autosave: Bool?, from captured: Capture) -> SavedView {
        SavedView(
            id: id,
            name: name,
            tickers: captured.configs.map(\.symbol),
            timeRange: captured.timeRange,
            createdAt: Date(),
            tickerConfigs: captured.configs,
            chartColumns: captured.columns,
            candleCount: captured.candleCount,
            autosave: autosave
        )
    }

    /// Persist `view` and make it the tab's clean baseline. On failure nothing changes: the tab keeps
    /// its layout and stays dirty.
    private func commit(
        _ view: SavedView, from captured: Capture, failure: String, reportErrors: Bool = true
    ) -> Bool {
        do {
            try store.upsert(view)
        } catch {
            if reportErrors { lastError = "Couldn’t \(failure): \(error.localizedDescription)" }
            return false
        }
        cancelAutosave()
        activeViewID = view.id
        baseline = captured.snapshot
        isDirty = false
        autosaveFailed = false
        onIdentityChange()
        return true
    }

    /// The active layout vanished (deleted here or in another tab): the tab keeps its charts and
    /// becomes Unnamed.
    private func detach() {
        cancelAutosave()
        activeViewID = nil
        baseline = .empty
        autosaveFailed = false
        refresh()
        onIdentityChange()
    }

    private func storeDidChange() {
        objectWillChange.send()
        if activeViewID != nil, activeView == nil {
            detach()
        } else {
            onIdentityChange()
        }
    }

    private func scheduleAutosave() {
        guard autosaveEnabled, let id = activeViewID else {
            cancelAutosave()
            return
        }
        pendingAutosave?.cancel()
        generation += 1
        let token = generation
        let sleep = sleep
        let delay = autosaveDelay
        pendingAutosave = Task { [weak self] in
            do { try await sleep(delay) } catch { return }
            guard !Task.isCancelled else { return }
            self?.fireAutosave(viewID: id, token: token)
        }
    }

    /// A delayed save only runs if the tab is still on the layout, and still in the debounce
    /// window, that scheduled it — so an autosave for layout A can never write layout B.
    private func fireAutosave(viewID: UUID, token: Int) {
        guard token == generation, viewID == activeViewID, isDirty, !isRestoring else { return }
        pendingAutosave = nil
        if !saveActive(reportErrors: false) { autosaveFailed = true }
    }

    private func cancelAutosave() {
        pendingAutosave?.cancel()
        pendingAutosave = nil
        generation += 1
    }
}
