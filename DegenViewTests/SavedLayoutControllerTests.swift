import XCTest

@testable import DegenView

@MainActor
final class SavedLayoutControllerTests: XCTestCase {
    private struct WriteFailure: Error {}

    /// Holds a sleeping autosave until the test lets it through.
    private actor Gate {
        private var continuation: CheckedContinuation<Void, Never>?
        private var isOpen = false
        func wait() async {
            if isOpen { return }
            await withCheckedContinuation { continuation = $0 }
        }
        func open() {
            isOpen = true
            continuation?.resume()
            continuation = nil
        }
    }

    private final class Clock {
        var time = Date(timeIntervalSince1970: 1_000)
        func tick() -> Date {
            time += 60
            return time
        }
    }

    /// A stand-in tab: the controller captures `state` and "rebuilds" it by assigning.
    @MainActor
    private final class Harness {
        let store: SavedViewStore
        let controller: SavedLayoutController
        var state: SavedLayoutController.Capture
        var failWrites = false

        init(
            activeViewID: UUID? = nil, seed: [SavedView] = [],
            sleep: @escaping (UInt64) async throws -> Void = { _ in }
        ) throws {
            let clock = Clock()
            let flag = FailFlag()
            store = SavedViewStore(
                database: try .makeInMemory(),
                persist: { _ in if flag.on { throw WriteFailure() } },
                now: { clock.tick() })
            for view in seed { try store.upsert(view) }
            controller = SavedLayoutController(store: store, activeViewID: activeViewID, sleep: sleep)
            self.flag = flag
            state = Harness.capture(of: store.view(id: activeViewID))
            controller.capture = { [unowned self] in state }
            controller.applyView = { [unowned self] in state = Harness.capture(of: $0) }
            controller.refresh()
        }

        private let flag: FailFlag
        var failing: Bool {
            get { flag.on }
            set { flag.on = newValue }
        }

        static func capture(of view: SavedView?) -> SavedLayoutController.Capture {
            guard let view else {
                return .init(timeRange: .oneDay, configs: [], columns: [], candleCount: TimeRange.oneDay.dataPointLimit)
            }
            return .init(
                timeRange: view.timeRange, configs: view.tickerConfigs,
                columns: ChartColumn.resolved(view.chartColumns, chartIDs: view.tickerConfigs.map(\.chartID)),
                candleCount: view.candleCount)
        }

        /// Append a chart the way the tab would: configs and columns change together.
        func addChart(_ symbol: String = "SOLUSDT") {
            let config = TickerConfig(symbol: symbol, source: .binance)
            state.configs.append(config)
            state.columns = ChartColumn.resolved(state.columns, chartIDs: state.configs.map(\.chartID))
        }
    }

    private final class FailFlag { var on = false }

    private func makeView(_ name: String, symbols: [String] = ["BTCUSDT", "ETHUSDT"]) -> SavedView {
        let configs = symbols.map { TickerConfig(symbol: $0, source: .binance) }
        return SavedView(
            name: name, tickers: symbols, timeRange: .oneDay, createdAt: Date(),
            tickerConfigs: configs, candleCount: TimeRange.oneDay.dataPointLimit)
    }

    // MARK: - Dirty state

    func testDirtyTracksPersistedConfigurationOnly() throws {
        let a = makeView("A")
        let h = try Harness(seed: [a])
        h.controller.requestOpen(a)
        XCTAssertEqual(h.controller.activeViewID, a.id)
        XCTAssertFalse(h.controller.isDirty, "a freshly opened layout is clean")

        h.addChart()
        h.controller.refresh()
        XCTAssertTrue(h.controller.isDirty)

        XCTAssertTrue(h.controller.saveActive())
        XCTAssertFalse(h.controller.isDirty)

        h.state.configs.removeLast()
        h.state.columns = ChartColumn.resolved(h.state.columns, chartIDs: h.state.configs.map(\.chartID))
        h.controller.refresh()
        XCTAssertTrue(h.controller.isDirty, "removing a chart")

        XCTAssertTrue(h.controller.saveActive())
        h.state.configs[0].showRSI = true
        h.controller.refresh()
        XCTAssertTrue(h.controller.isDirty, "changing a chart setting")
    }

    func testRuntimeAndZoomChangesStayClean() throws {
        let a = makeView("A")
        let h = try Harness(seed: [a])
        h.controller.requestOpen(a)

        h.state.candleCount += 40  // scroll-zoom
        h.controller.refresh()
        XCTAssertFalse(h.controller.isDirty)
    }

    func testSaveKeepsIdentityAndUpdatesContent() throws {
        let a = makeView("A")
        let h = try Harness(seed: [a])
        h.controller.requestOpen(a)
        h.addChart()
        h.controller.refresh()

        h.controller.requestSave()

        XCTAssertNil(h.controller.prompt)
        XCTAssertEqual(h.store.views.count, 1)
        XCTAssertEqual(h.store.view(id: a.id)?.tickerConfigs.count, 3)
        XCTAssertFalse(h.controller.isDirty)
    }

    // MARK: - Unnamed

    func testUnnamedWithChartsIsDirtyAndFirstSaveNamesIt() throws {
        let h = try Harness()
        XCTAssertEqual(h.controller.displayName, UI.unnamedView)
        XCTAssertFalse(h.controller.isDirty)

        h.addChart()
        h.controller.refresh()
        XCTAssertTrue(h.controller.isDirty)

        h.controller.requestSave()
        XCTAssertEqual(h.controller.prompt, .save(then: nil))
        XCTAssertTrue(h.store.views.isEmpty, "asking for a name must not create a layout")

        h.controller.confirmPrompt(name: "My Layout")
        let id = try XCTUnwrap(h.controller.activeViewID)
        XCTAssertEqual(h.store.view(id: id)?.name, "My Layout")
        XCTAssertEqual(h.controller.displayName, "My Layout")
        XCTAssertFalse(h.controller.isDirty)
    }

    func testAutosaveIsUnavailableForUnnamed() throws {
        let h = try Harness()
        h.addChart()
        h.controller.setAutosave(true)
        h.controller.refresh()
        XCTAssertNil(h.controller.pendingAutosave)
        XCTAssertTrue(h.store.views.isEmpty)
    }

    // MARK: - Rename / copy

    func testRenameKeepsIdentityAndRecency() throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        let openedAt = h.store.view(id: a.id)?.lastOpenedAt

        XCTAssertTrue(h.controller.rename(to: "B"))

        XCTAssertEqual(h.controller.activeViewID, a.id)
        XCTAssertEqual(h.store.view(id: a.id)?.name, "B")
        XCTAssertEqual(h.store.view(id: a.id)?.lastOpenedAt, openedAt)
    }

    func testRenameFailureKeepsOldName() throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        h.failing = true
        XCTAssertFalse(h.controller.rename(to: "B"))
        XCTAssertEqual(h.controller.displayName, "A")
        XCTAssertNotNil(h.controller.lastError)
    }

    func testCopyGetsNewIdentityAndCurrentUnsavedState() throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        h.addChart()
        h.controller.refresh()

        XCTAssertTrue(h.controller.makeCopy(named: "A copy"))

        let copyID = try XCTUnwrap(h.controller.activeViewID)
        XCTAssertNotEqual(copyID, a.id)
        XCTAssertEqual(h.store.view(id: copyID)?.tickerConfigs.count, 3)
        XCTAssertEqual(h.store.view(id: a.id)?.tickerConfigs.count, 2, "the original keeps its saved content")
        XCTAssertFalse(h.controller.isDirty)
    }

    func testCopyFailureKeepsActiveLayout() throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        h.failing = true
        XCTAssertFalse(h.controller.makeCopy(named: "A copy"))
        XCTAssertEqual(h.controller.activeViewID, a.id)
        XCTAssertEqual(h.store.views.count, 1)
    }

    func testSaveFailureKeepsDirty() throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        h.addChart()
        h.controller.refresh()
        h.failing = true

        XCTAssertFalse(h.controller.saveActive())
        XCTAssertTrue(h.controller.isDirty)
        XCTAssertNotNil(h.controller.lastError)
    }

    // MARK: - Recents

    func testRecentsFollowOpenOrder() throws {
        let views = ["A", "B", "C"].map { makeView($0) }
        let h = try Harness(seed: views)
        // Seeding stamps each view, so open them explicitly in the order under test.
        views.forEach { h.controller.requestOpen($0) }

        XCTAssertEqual(h.controller.activeViewID, views[2].id)
        XCTAssertEqual(h.controller.recentViews.map(\.name), ["B", "A"])

        h.controller.delete(views[1].id)
        XCTAssertEqual(h.controller.recentViews.map(\.name), ["A"])
    }

    // MARK: - Switching

    private func dirtyOnA() throws -> (Harness, SavedView, SavedView) {
        let a = makeView("A")
        let b = makeView("B", symbols: ["XRPUSDT"])
        let h = try Harness(seed: [a, b])
        h.controller.requestOpen(a)
        h.addChart()
        h.controller.refresh()
        return (h, a, b)
    }

    func testSwitchingWhileDirtyAsksFirst() throws {
        let (h, a, b) = try dirtyOnA()
        h.controller.requestOpen(b)
        XCTAssertEqual(h.controller.pendingTransition, .open(b.id))
        XCTAssertEqual(h.controller.activeViewID, a.id)
    }

    func testCancelStaysOnTheCurrentLayout() throws {
        let (h, a, b) = try dirtyOnA()
        h.controller.requestOpen(b)
        h.controller.resolve(.cancel)
        XCTAssertEqual(h.controller.activeViewID, a.id)
        XCTAssertEqual(h.state.configs.count, 3)
        XCTAssertTrue(h.controller.isDirty)
    }

    func testDontSaveDiscardsAndSwitches() throws {
        let (h, a, b) = try dirtyOnA()
        h.controller.requestOpen(b)
        h.controller.resolve(.discard)
        XCTAssertEqual(h.controller.activeViewID, b.id)
        XCTAssertEqual(h.store.view(id: a.id)?.tickerConfigs.count, 2)
        XCTAssertFalse(h.controller.isDirty)
    }

    func testSaveThenSwitches() throws {
        let (h, a, b) = try dirtyOnA()
        h.controller.requestOpen(b)
        h.controller.resolve(.save)
        XCTAssertEqual(h.store.view(id: a.id)?.tickerConfigs.count, 3)
        XCTAssertEqual(h.controller.activeViewID, b.id)
    }

    func testFailedSaveDoesNotSwitch() throws {
        let (h, a, b) = try dirtyOnA()
        h.controller.requestOpen(b)
        h.failing = true
        h.controller.resolve(.save)
        XCTAssertEqual(h.controller.activeViewID, a.id)
        XCTAssertTrue(h.controller.isDirty)
    }

    func testSavingAnUnnamedTabBeforeSwitchingChainsTheNamePrompt() throws {
        let b = makeView("B")
        let h = try Harness(seed: [b])
        h.addChart()
        h.controller.refresh()

        h.controller.requestOpen(b)
        h.controller.resolve(.save)
        XCTAssertEqual(h.controller.prompt, .save(then: .open(b.id)))

        h.controller.confirmPrompt(name: "Mine")
        XCTAssertEqual(h.store.views.count, 2)
        XCTAssertEqual(h.controller.activeViewID, b.id)
    }

    // MARK: - Autosave

    func testAutosaveOffLeavesChangesUnsaved() throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        h.addChart()
        h.controller.refresh()
        XCTAssertTrue(h.controller.isDirty)
        XCTAssertNil(h.controller.pendingAutosave)
        XCTAssertEqual(h.store.view(id: a.id)?.tickerConfigs.count, 2)
        XCTAssertTrue(h.controller.showsSaveAffordance)
    }

    func testAutosaveOnSavesAfterTheDebounce() async throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        h.controller.setAutosave(true)
        h.addChart()
        h.controller.refresh()
        XCTAssertFalse(h.controller.showsSaveAffordance, "autosave keeps the toolbar quiet")

        await h.controller.pendingAutosave?.value

        XCTAssertEqual(h.store.view(id: a.id)?.tickerConfigs.count, 3)
        XCTAssertFalse(h.controller.isDirty)
    }

    func testAutosaveFailureStaysDirtyWithoutRetrying() async throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        h.controller.setAutosave(true)
        h.failing = true
        h.addChart()
        h.controller.refresh()

        await h.controller.pendingAutosave?.value

        XCTAssertTrue(h.controller.isDirty)
        XCTAssertTrue(h.controller.autosaveFailed)
        XCTAssertTrue(h.controller.showsSaveAffordance)
        XCTAssertNil(h.controller.lastError, "autosave failures don't interrupt with an alert")
        XCTAssertNil(h.controller.pendingAutosave, "no retry until the next change")
    }

    func testPendingAutosaveIsFlushedAndNeverWritesTheNextLayout() async throws {
        let gate = Gate()
        let a = makeView("A")
        let b = makeView("B", symbols: ["XRPUSDT"])
        let h = try Harness(activeViewID: nil, seed: [a, b], sleep: { _ in await gate.wait() })
        h.controller.requestOpen(a)
        h.controller.setAutosave(true)
        h.addChart()
        h.controller.refresh()
        let stale = try XCTUnwrap(h.controller.pendingAutosave)

        h.controller.requestOpen(b)  // flushes A, then switches

        XCTAssertEqual(h.store.view(id: a.id)?.tickerConfigs.count, 3, "flushed before switching")
        XCTAssertEqual(h.controller.activeViewID, b.id)

        await gate.open()
        await stale.value

        XCTAssertEqual(h.store.view(id: b.id)?.tickerConfigs.count, 1, "the old timer never touches B")
        XCTAssertFalse(h.controller.isDirty)
    }

    func testDeletingTheActiveLayoutLeavesAnUnnamedTab() throws {
        let a = makeView("A")
        let h = try Harness(activeViewID: a.id, seed: [a])
        h.controller.delete(a.id)
        XCTAssertNil(h.controller.activeViewID)
        XCTAssertEqual(h.controller.displayName, UI.unnamedView)
        XCTAssertTrue(h.controller.isDirty, "its charts now have no saved home")
    }
}
