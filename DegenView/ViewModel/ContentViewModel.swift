import Foundation
import SwiftUI
import AppKit

enum LayoutMode: String, CaseIterable, Codable {
    case vertical
    case grid

    var icon: String {
        switch self {
        case .vertical: return "rectangle.stack"
        case .grid:     return "rectangle.grid.1x2"
        }
    }

    var next: LayoutMode {
        switch self {
        case .vertical: return .grid
        case .grid:     return .vertical
        }
    }
}

/// State for one tab. Every window owns exactly one of these, hydrated from the
/// `ChartTab` that `TabsStore` holds for its id and written back on every change.
@MainActor
final class ContentViewModel: ObservableObject {
    /// Which tab this view model drives — also the key for its slice of the
    /// CoinGecko prime and its entry in `WindowCoordinator`.
    let tabID: UUID
    let replay = ReplayEngine()
    private var pendingReplayRestore: ReplaySession?
    @Published private(set) var isPreparingReplay = false
    @Published var replayNotice: String?
    private var replayPreparationTask: Task<Void, Never>?

    @Published var chartViewModels: [ChartViewModel] = []
    var marketChartViewModels: [ChartViewModel] { chartViewModels.filter { !$0.isPortfolioChart } }
    @Published var selectedTimeRange: TimeRange = .oneDay {
        didSet {
            // Reset candle count to default when timeframe changes
            if oldValue != selectedTimeRange {
                candleCount = selectedTimeRange.dataPointLimit
                // The rubber band of a half-drawn line tracks the pointer, which is
                // over the toolbar rather than the plot — drop it rather than leave
                // it stretched across a chart that just changed under it. The crosshair
                // goes for the same reason: its time is no longer on screen, and so does
                // any measurement, which was taken against the candles just replaced.
                for vm in chartViewModels {
                    vm.cancelDraft()
                    vm.cancelRulerDraft()
                    vm.clearRulers()
                }
                crosshair.clear()
            }
        }
    }
    @Published var candleCount: Int = TimeRange.oneDay.dataPointLimit
    @Published var layoutMode: LayoutMode = .vertical {
        didSet { markChanged() }
    }
    @Published var isRefreshing = false

    @Published var savedViews: [SavedView] = []

    /// The tab's label. Shows in the name bar *and* as the window title, which on
    /// macOS is what the system tab bar draws.
    @Published var tabName = UI.unnamedView {
        didSet {
            guard oldValue != tabName else { return }
            WindowCoordinator.shared.syncTitle(for: tabID)
        }
    }
    @Published var hasUnsavedChanges = false

    private var currentViewID: UUID?
    private var isApplyingView = false
    /// Set while `init` assigns the tab's fields. `layoutMode`'s `didSet` would
    /// otherwise write the tab back before `chartViewModels` is populated,
    /// erasing the very charts being restored.
    private var isHydrating = true

    private let api: BinanceAPIService
    private let viewStore = JSONStore<[SavedView]>(filename: "views.json")
    private var refreshTimer: Timer?
    private let wsService = BinanceWebSocketService()
    private let alpacaWSService = AlpacaWebSocketService()

    private var scrollMonitor: Any?
    private var pendingZoomDelta = 0
    private var zoomDebounceTask: Task<Void, Never>?
    /// Card rectangles a scroll has to land in to count as a zoom. Weak, so a
    /// removed card's marker view takes its entry with it.
    private let zoomRegions = NSHashTable<NSView>.weakObjects()

    private var mouseMonitor: Any?
    /// Y-axis gutters, each mapped to the chart it scales. Weak on both sides, so a
    /// removed card drops out on its own.
    private let axisRegions = NSMapTable<NSView, ChartViewModel>.weakToWeakObjects()
    /// The chart whose axis is being dragged right now, and where the drag started.
    private var axisDragTarget: ChartViewModel?
    private var axisDragOrigin: NSPoint = .zero
    private var axisDidDrag = false

    /// Plot areas, each mapped to the chart drawn in it. Weak on both sides, so a
    /// removed card drops out on its own.
    private let plotRegions = NSMapTable<NSView, ChartViewModel>.weakToWeakObjects()
    /// The endpoint being dragged right now, and the chart it belongs to.
    private var lineDragTarget: (vm: ChartViewModel, id: UUID, isStart: Bool)?

    /// The tab's crosshair. Its own observable object rather than a `@Published` here —
    /// see `CrosshairTracker`.
    let crosshair = CrosshairTracker()

    /// Which tool the tool strip has armed. Window-scoped: arming in one tab leaves the
    /// others alone. The crosshair starts armed — it only reads, so a tab opens ready to
    /// take a measurement and Esc still puts it away.
    @Published var activeTool: ChartTool = .crosshair {
        didSet {
            guard activeTool != oldValue else { return }
            // The rubber band runs off mouse-moved events, and AppKit only posts
            // those to a window that has asked for them.
            ownWindow?.acceptsMouseMovedEvents = activeTool != .none
            crosshair.clear()
            // A measurement belongs to the armed ruler — reaching for another tool is
            // done with it. Trend lines, being annotations, stay.
            for vm in chartViewModels {
                vm.cancelRulerDraft()
                vm.clearRulers()
            }
            guard activeTool == .none else { return }
            for vm in chartViewModels {
                vm.cancelDraft()
                vm.selectedLineID = nil
                vm.editingLineID = nil
            }
        }
    }

    /// Suppress scroll-zoom when sheets or popovers are presented.
    var isShowingSheet = false
    /// The inline trend-line editor blocks chart gestures, but still accepts deletion keys.
    var isShowingLineEditor = false

    // MARK: - Window binding

    private weak var ownWindow: NSWindow?
    private var observers: [NSObjectProtocol] = []
    /// Hidden tabs don't poll — see `updateVisibility(_:)`.
    private var isWindowVisible = false
    private var didInitialLoad = false

    init(tabID: UUID, api: BinanceAPIService = BinanceAPIService()) {
        self.tabID = tabID
        self.api = api
        self.savedViews = viewStore.load() ?? []

        let tab = TabsStore.shared.ensureTab(tabID)
        pendingReplayRestore = tab.replaySession
        tabName = tab.name
        currentViewID = tab.savedViewID
        selectedTimeRange = tab.timeRange
        // Assigned after the range, whose didSet would otherwise reset it.
        candleCount = tab.candleCount
        layoutMode = tab.layoutMode
        chartViewModels = tab.tickerConfigs.map { config in
            let vm = ChartViewModel(ticker: config.symbol, source: config.source)
            vm.applyConfig(config)
            return vm
        }
        hasUnsavedChanges = false
        isHydrating = false

        scrollMonitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
            guard let self, !self.chartViewModels.isEmpty, !self.isShowingSheet else { return event }
            // A local monitor sees every scroll in the app, so without this each
            // open tab would zoom on a scroll aimed at one of the others.
            guard let own = self.ownWindow, event.window === own else { return event }
            guard self.pointerIsOverChart(event) else { return event }
            let step = max(1, Int(Double(self.candleCount) * Candle.zoomStepFraction))
            if event.scrollingDeltaY > 0 {
                self.pendingZoomDelta += step
            } else if event.scrollingDeltaY < 0 {
                self.pendingZoomDelta -= step
            } else {
                return event
            }
            self.zoomDebounceTask?.cancel()
            self.zoomDebounceTask = Task { [weak self] in
                try? await Task.sleep(nanoseconds: 150_000_000) // 150 ms
                guard let self else { return }
                let delta = self.pendingZoomDelta
                self.pendingZoomDelta = 0
                guard delta != 0 else { return }
                self.adjustCandleCount(by: delta)
            }
            return event
        }

        mouseMonitor = NSEvent.addLocalMonitorForEvents(
            matching: [.leftMouseDown, .leftMouseDragged, .leftMouseUp, .mouseMoved, .keyDown]
        ) { [weak self] event in
            guard let self else { return event }
            if self.replay.status == .selectingStart {
                return self.handleReplaySelection(event)
            }
            // The price gutter gets first refusal; a swallowed event never reaches
            // the drawing tool, which only ever acts inside the plot anyway.
            guard let remaining = self.handleAxisDrag(event) else { return nil }
            return self.handleDrawing(remaining)
        }

        replay.onStateChange = { [weak self] in
            self?.applyReplayState()
        }
    }

    deinit {
        if let m = scrollMonitor { NSEvent.removeMonitor(m) }
        if let m = mouseMonitor { NSEvent.removeMonitor(m) }
    }

    // MARK: - Scroll-zoom targeting

    /// Each chart card hands over the view covering it, so the window-wide
    /// monitor can tell a zoom from a scroll aimed anywhere else.
    func registerZoomRegion(_ view: NSView) {
        zoomRegions.add(view)
    }

    /// `bounds`, not `visibleRect`: SwiftUI's superviews don't clip their
    /// subviews, so a card's `visibleRect` covers the whole window and matches
    /// every scroll. The toolbar strip is excluded separately — a card scrolled
    /// up under it still sits at window coordinates the toolbar draws over.
    private func pointerIsOverChart(_ event: NSEvent) -> Bool {
        guard let window = event.window, let content = window.contentView else { return false }
        let point = event.locationInWindow
        guard window.contentLayoutRect.contains(content.convert(point, from: nil)) else { return false }
        return zoomRegions.allObjects.contains { region in
            guard region.window === window, !region.isHiddenOrHasHiddenAncestor else { return false }
            return region.bounds.contains(region.convert(point, from: nil))
        }
    }

    // MARK: - Y-axis drag zoom

    /// Each chart card hands over the view covering its price-axis gutter, paired
    /// with the chart that gutter belongs to.
    func registerAxisRegion(_ view: NSView, for viewModel: ChartViewModel) {
        axisRegions.setObject(viewModel, forKey: view)
    }

    /// Drag the price axis to scale it: up for a narrower price slice (taller
    /// candles), down for a wider one. Double-click restores auto-fit.
    ///
    /// Runs off a monitor rather than an `NSView`'s mouse handlers because SwiftUI's
    /// hosting view claims these events for the cards' `.onDrag` reordering before
    /// AppKit offers them to any child view. Returning nil for a gesture that lands
    /// on a gutter is what keeps that reorder drag from starting.
    private func handleAxisDrag(_ event: NSEvent) -> NSEvent? {
        guard !isShowingSheet, let own = ownWindow, event.window === own else { return event }

        switch event.type {
        case .leftMouseDown:
            guard let target = axisRegion(at: event) else { return event }
            guard event.clickCount < 2 else {
                axisDragTarget = nil
                target.resetYZoom()
                persistChartSettings()
                return nil
            }
            axisDragTarget = target
            // Window coordinates are y-up, so the delta is already "positive = up".
            axisDragOrigin = event.locationInWindow
            axisDidDrag = false
            target.beginYZoomDrag()
            return nil

        case .leftMouseDragged:
            guard let target = axisDragTarget else { return event }
            axisDidDrag = true
            target.updateYZoom(dragOffset: event.locationInWindow.y - axisDragOrigin.y)
            return nil

        case .leftMouseUp:
            guard axisDragTarget != nil else { return event }
            axisDragTarget = nil
            // One persist per gesture, not one per mouse-move.
            if axisDidDrag { persistChartSettings() }
            axisDidDrag = false
            return nil

        default:
            return event
        }
    }

    /// The chart whose price-axis gutter sits under this event, if any.
    private func axisRegion(at event: NSEvent) -> ChartViewModel? {
        guard let window = event.window, let content = window.contentView else { return nil }
        let point = event.locationInWindow
        guard window.contentLayoutRect.contains(content.convert(point, from: nil)) else { return nil }
        guard let views = axisRegions.keyEnumerator().allObjects as? [NSView] else { return nil }
        for view in views {
            guard view.window === window, !view.isHiddenOrHasHiddenAncestor else { continue }
            if view.bounds.contains(view.convert(point, from: nil)) {
                return axisRegions.object(forKey: view)
            }
        }
        return nil
    }

    // MARK: - Trend-line drawing

    /// Each chart card hands over the view covering its plot, paired with the chart
    /// drawn in it.
    func registerPlotRegion(_ view: NSView, for viewModel: ChartViewModel) {
        plotRegions.setObject(viewModel, forKey: view)
    }

    // MARK: - Historical replay

    func beginReplaySelection() {
        guard !chartViewModels.isEmpty else { return }
        replay.beginSelecting()
        activeTool = .none
        crosshair.clear()
        wsService.disconnect()
        alpacaWSService.disconnect()
    }

    func selectReplayStart(_ date: Date) {
        guard let primary = marketChartViewModels.first, !primary.klineData.isEmpty else { return }
        clearReplaySelectionMarkers()
        replayPreparationTask?.cancel()
        replayPreparationTask = Task { [weak self] in
            guard let self else { return }
            self.isPreparingReplay = true
            self.replayNotice = nil
            defer { self.isPreparingReplay = false }
            await self.prepareGranularReplay(interval: .automatic)
            guard !Task.isCancelled, let primary = self.marketChartViewModels.first else { return }
            let timeline = primary.replayTimeline()
            self.replay.start(
                at: date,
                initialCursorAt: primary.granularReplayInterval == nil
                    ? date : primary.replayBarEnd(containing: date),
                symbol: primary.apiSymbol,
                timeframe: self.selectedTimeRange,
                timeline: timeline
            )
            self.replay.setInterval(.automatic)
        }
    }

    func selectFirstReplayBar() {
        guard let date = marketChartViewModels.first?.klineData.first?.openTime else { return }
        selectReplayStart(date)
    }

    func selectRandomReplayBar() {
        guard let data = marketChartViewModels.first?.klineData, data.count > 1 else { return }
        // Reserve at least 20% (and at least one bar) for useful future progression.
        let reserve = max(1, min(data.count - 1, max(5, data.count / 5)))
        selectReplayStart(data[Int.random(in: 0..<(data.count - reserve))].openTime)
    }

    func selectReplayDate(_ date: Date) { selectReplayStart(date) }

    func returnToLive() {
        replayPreparationTask?.cancel()
        replay.jumpToLatest()
        chartViewModels.forEach { $0.clearGranularReplayData() }
        clearReplaySelectionMarkers()
        if isWindowVisible { connectWebSocket() }
        refetchAll()
    }

    var availableReplayIntervals: [ReplayInterval] {
        marketChartViewModels.first?.supportedReplayIntervals ?? [.automatic, .chartBar]
    }

    func setReplayInterval(_ interval: ReplayInterval) {
        guard replay.session != nil else { return }
        replay.pause()
        replayPreparationTask?.cancel()
        replayPreparationTask = Task { [weak self] in
            guard let self else { return }
            self.isPreparingReplay = true
            self.replayNotice = nil
            defer { self.isPreparingReplay = false }
            await self.prepareGranularReplay(interval: interval)
            guard !Task.isCancelled, let primary = self.marketChartViewModels.first else { return }
            self.replay.setInterval(interval)
            self.replay.updateTimeline(
                primary.replayTimeline(),
                symbol: primary.apiSymbol,
                timeframe: self.selectedTimeRange
            )
        }
    }

    private func prepareGranularReplay(interval: ReplayInterval) async {
        var failures: [String] = []
        for vm in chartViewModels {
            guard !Task.isCancelled else { return }
            do {
                let resolved = try await vm.loadGranularReplayData(interval: interval)
                if interval != .chartBar, resolved == .chartBar {
                    failures.append(vm.title)
                }
            } catch {
                vm.clearGranularReplayData()
                failures.append(vm.title)
            }
        }
        if !failures.isEmpty {
            replayNotice = "Granular history unavailable for \(failures.joined(separator: ", ")); replaying complete chart bars."
        }
    }

    private func applyReplayState() {
        let timestamp = replay.currentTimestamp
        for vm in chartViewModels {
            vm.applyReplayTimestamp(replay.isActive && replay.status != .selectingStart ? timestamp : nil)
        }
        syncTab()
    }

    private func clearReplaySelectionMarkers() {
        chartViewModels.forEach { $0.applyReplaySelectionTimestamp(nil) }
    }

    private func handleReplaySelection(_ event: NSEvent) -> NSEvent? {
        if event.type == .keyDown {
            guard event.keyCode == 53 else { return event }
            replay.cancelSelection()
            clearReplaySelectionMarkers()
            return nil
        }
        guard let hit = plotHit(at: event),
              let date = hit.vm.replayDate(nearestTo: hit.point, in: hit.plot)
        else {
            if event.type == .mouseMoved { clearReplaySelectionMarkers() }
            return event
        }
        if event.type == .mouseMoved {
            clearReplaySelectionMarkers()
            hit.vm.applyReplaySelectionTimestamp(date)
            return event
        }
        if event.type == .leftMouseDown {
            selectReplayStart(date)
            return nil
        }
        return event.type == .leftMouseUp ? nil : event
    }

    /// Toggle a tool on the strip; clicking the armed one disarms it.
    func toggleTool(_ tool: ChartTool) {
        activeTool = activeTool == tool ? .none : tool
    }

    /// Click once to start a line, again to finish it; drag either end circle to move
    /// it. Nothing here runs unless the tool is armed, so a disarmed window behaves
    /// exactly as it did before drawing existed.
    ///
    /// Shares the mouse monitor with the axis-zoom drag for the same reason it exists:
    /// SwiftUI's hosting view claims these events for the cards' `.onDrag` reordering
    /// before AppKit offers them to any child view. Returning nil for a press inside a
    /// plot is what keeps a reorder drag from starting under the pointer.
    private func handleDrawing(_ event: NSEvent) -> NSEvent? {
        guard let own = ownWindow, event.window === own else { return event }
        if event.type == .keyDown,
           event.keyCode == 51 || event.keyCode == 117,
           !isShowingSheet,
           isShowingLineEditor {
            return handleDrawingKey(event)
        }
        guard !isShowingSheet, !isShowingLineEditor else { return event }
        if activeTool == .crosshair { return handleCrosshair(event) }
        if activeTool == .ruler { return handleRuler(event) }
        guard activeTool == .trendLine else { return event }

        if event.type == .keyDown { return handleDrawingKey(event) }

        // A handle drag owns the pointer until release, wherever it wanders.
        if let target = lineDragTarget {
            switch event.type {
            case .leftMouseDragged:
                if let hit = plotHit(at: event) {
                    target.vm.moveAnchor(
                        lineID: target.id,
                        isStart: target.isStart,
                        to: target.vm.anchor(at: hit.point, in: hit.plot)
                    )
                }
                return nil
            case .leftMouseUp:
                target.vm.persistTrendLines()
                lineDragTarget = nil
                return nil
            default:
                break
            }
        }

        guard let hit = plotHit(at: event) else { return event }
        let anchor = hit.vm.anchor(at: hit.point, in: hit.plot)

        switch event.type {
        case .mouseMoved:
            // Rubber band only. Never swallowed — the pointer still belongs to the app.
            hit.vm.updateDraft(to: anchor)
            return event

        case .leftMouseDown:
            // A click on another card abandons whatever was half-drawn there, rather
            // than leaving a dangling rubber band behind.
            clearDrawingState(except: hit.vm)

            if hit.vm.hasDraft {
                _ = hit.vm.commitDraft(at: anchor, in: hit.plot)
            } else if let handle = hit.vm.handleHit(at: hit.point, in: hit.plot) {
                lineDragTarget = (hit.vm, handle.id, handle.isStart)
                hit.vm.selectedLineID = handle.id
                hit.vm.editingLineID = nil
            } else if let line = hit.vm.lineHit(at: hit.point, in: hit.plot) {
                hit.vm.selectedLineID = line
                hit.vm.editingLineID = line
            } else {
                hit.vm.selectedLineID = nil
                hit.vm.editingLineID = nil
                hit.vm.beginDraft(at: anchor)
            }
            return nil

        case .leftMouseUp:
            return nil

        default:
            return event
        }
    }

    /// Click once to pin a corner, again to finish the rectangle, a third time to put it
    /// away. Same click-move-click shape as the line tool, minus the parts a measurement
    /// doesn't need: nothing to select, nothing to drag, nothing to persist.
    private func handleRuler(_ event: NSEvent) -> NSEvent? {
        if event.type == .keyDown { return handleRulerKey(event) }

        guard let hit = plotHit(at: event) else { return event }
        let anchor = hit.vm.anchor(at: hit.point, in: hit.plot)

        switch event.type {
        case .mouseMoved:
            // Rubber band only. Never swallowed — the pointer still belongs to the app.
            hit.vm.updateRulerDraft(to: anchor)
            return event

        case .leftMouseDown:
            // A click on another card abandons whatever was half-drawn there, but leaves
            // its finished measurement up — two charts can be read side by side.
            clearDrawingState(except: hit.vm)

            if hit.vm.hasRulerDraft {
                hit.vm.commitRulerDraft(at: anchor, in: hit.plot)
            } else if !hit.vm.clearRulers() {
                // Nothing was up to dismiss, so this click starts a rectangle instead.
                hit.vm.beginRulerDraft(at: anchor)
            }
            return nil

        case .leftMouseUp:
            return nil

        default:
            return event
        }
    }

    /// Esc backs out of a half-drawn rectangle, then out of the measurements on screen,
    /// then out of the tool. Delete clears the measurements outright.
    private func handleRulerKey(_ event: NSEvent) -> NSEvent? {
        switch event.keyCode {
        case 53:  // Esc
            if let drafting = chartViewModels.first(where: { $0.hasRulerDraft }) {
                drafting.cancelRulerDraft()
            } else if !clearAllRulers() {
                activeTool = .none
            }
            return nil

        case 51, 117:  // Delete, forward delete
            return clearAllRulers() ? nil : event

        default:
            return event
        }
    }

    /// Puts every chart's measurements away, reporting whether any were up. Not written
    /// as a `filter`, which would hide a side effect in a predicate.
    @discardableResult
    private func clearAllRulers() -> Bool {
        var cleared = false
        for vm in chartViewModels {
            if vm.clearRulers() { cleared = true }
        }
        return cleared
    }

    /// Tracks the pointer for the crosshair tool.
    ///
    /// Read-only, so unlike the trend-line tool it swallows nothing but Esc: clicks keep
    /// reaching the cards, and drag-reorder and the gutter drag work while it is armed.
    ///
    /// A pointer that leaves the window posts no further mouse-moved event, so this can't
    /// be the only thing that clears the crosshair — `ChartCardView`'s `onHover` handles
    /// the exit.
    private func handleCrosshair(_ event: NSEvent) -> NSEvent? {
        switch event.type {
        case .keyDown:
            guard event.keyCode == 53 else { return event }  // Esc
            activeTool = .none
            return nil

        case .mouseMoved:
            guard let hit = plotHit(at: event) else {
                crosshair.clear()
                return event
            }
            let rect = hit.plot.plotRect
            guard rect.width > 0 else { return event }
            crosshair.update(
                Crosshair(
                    ownerID: hit.vm.uniqueID,
                    xFraction: (hit.point.x - rect.minX) / rect.width,
                    price: hit.plot.price(forY: hit.point.y)
                )
            )
            return event

        default:
            return event
        }
    }

    /// Esc backs out of a half-drawn line, then out of the tool. Delete removes the
    /// selected line. Anything else passes through untouched.
    private func handleDrawingKey(_ event: NSEvent) -> NSEvent? {
        switch event.keyCode {
        case 53:  // Esc
            if let drafting = chartViewModels.first(where: { $0.hasDraft }) {
                drafting.cancelDraft()
            } else {
                activeTool = .none
            }
            return nil

        case 51, 117:  // Delete, forward delete
            guard let target = chartViewModels.first(where: { $0.selectedLineID != nil }) else {
                return event
            }
            _ = target.removeSelectedLine()
            return nil

        default:
            return event
        }
    }

    /// The chart under this event, its current geometry, and where the pointer landed
    /// in that chart's canvas space.
    private func plotHit(at event: NSEvent) -> (vm: ChartViewModel, plot: ChartPlot, point: CGPoint)? {
        guard let window = event.window, let content = window.contentView else { return nil }
        let location = event.locationInWindow
        guard window.contentLayoutRect.contains(content.convert(location, from: nil)) else { return nil }
        guard let views = plotRegions.keyEnumerator().allObjects as? [NSView] else { return nil }

        for view in views {
            guard view.window === window, !view.isHiddenOrHasHiddenAncestor else { continue }
            // The region view is flipped, so this is already the canvas' own space.
            let point = view.convert(location, from: nil)
            guard view.bounds.contains(point), let vm = plotRegions.object(forKey: view) else { continue }
            let plot = vm.plot(in: view.bounds.size)
            // The trailing gutter overlaps this region but belongs to the axis drag.
            guard plot.plotRect.contains(point) else { return nil }
            return (vm, plot, point)
        }
        return nil
    }

    /// Drops what the other charts had in progress. Their finished rectangles stay —
    /// clearing a measurement is per-chart, so a click here can't wipe the one next door.
    private func clearDrawingState(except keep: ChartViewModel) {
        for vm in chartViewModels where vm !== keep {
            vm.cancelDraft()
            vm.cancelRulerDraft()
            vm.selectedLineID = nil
            vm.editingLineID = nil
        }
    }

    /// Bind to the hosting window: scope the scroll monitor, follow occlusion,
    /// and tear down when the tab is closed for real.
    func attach(to window: NSWindow) {
        guard ownWindow !== window else { return }
        ownWindow = window
        // `activeTool`'s observer can't do this for the tool armed at launch — there was
        // no window to ask when it was assigned.
        window.acceptsMouseMovedEvents = activeTool != .none
        WindowCoordinator.shared.register(window, for: tabID)

        observers.append(
            NotificationCenter.default.addObserver(
                forName: NSWindow.didChangeOcclusionStateNotification,
                object: window,
                queue: .main
            ) { [weak self, weak window] _ in
                guard let window else { return }
                let visible = window.occlusionState.contains(.visible)
                Task { @MainActor [weak self] in self?.updateVisibility(visible) }
            }
        )

        observers.append(
            NotificationCenter.default.addObserver(
                forName: NSWindow.willCloseNotification,
                object: window,
                queue: .main
            ) { [weak self] _ in
                Task { @MainActor [weak self] in self?.teardown() }
            }
        )

        updateVisibility(window.occlusionState.contains(.visible))
    }

    /// Only the tab the user is looking at polls. A background tab's window is
    /// occluded, which covers both tab switching and minimizing.
    private func updateVisibility(_ visible: Bool) {
        guard visible != isWindowVisible else { return }
        isWindowVisible = visible

        guard visible else {
            suspend()
            return
        }

        startAutoRefresh()
        connectWebSocket()

        if !didInitialLoad {
            didInitialLoad = true
            Task { await initialFetch() }
        } else {
            Task { await refetchAllSilent() }
        }
    }

    /// Stop network work while the tab is hidden.
    private func suspend() {
        refreshTimer?.invalidate()
        refreshTimer = nil
        wsService.disconnect()
        alpacaWSService.disconnect()
        refetchTask?.cancel()
    }

    /// Release everything this tab holds — its window is closing for good.
    private func teardown() {
        suspend()
        if let m = scrollMonitor {
            NSEvent.removeMonitor(m)
            scrollMonitor = nil
        }
        if let m = mouseMonitor {
            NSEvent.removeMonitor(m)
            mouseMonitor = nil
        }
        observers.forEach { NotificationCenter.default.removeObserver($0) }
        observers.removeAll()
        zoomDebounceTask?.cancel()

        let id = tabID
        Task {
            guard let cg = DataSourceFactory.shared.service(for: .coingecko) as? CoinGeckoAPIService else { return }
            await cg.clearActiveSymbols(for: id)
        }
        WindowCoordinator.shared.unregister(id)
    }

    /// Stagger the first paint so a full tab doesn't fire every request at once.
    private func initialFetch() async {
        await syncCoinGeckoSymbols()
        for vm in chartViewModels {
            guard !Task.isCancelled else { return }
            await vm.fetchData(for: selectedTimeRange, count: candleCount)
            try? await Task.sleep(nanoseconds: Timeout.fetchStaggerNS)
        }
        if let saved = pendingReplayRestore, let primary = marketChartViewModels.first {
            await prepareGranularReplay(interval: saved.replayInterval)
            replay.restore(saved, timeline: primary.replayTimeline())
            pendingReplayRestore = nil
        }
        connectWebSocket()
    }

    /// Refresh all charts every 5 seconds. Cache prevents redundant API calls.
    /// No loading indicator — silent background refresh. Runs only while visible.
    private func startAutoRefresh() {
        refreshTimer?.invalidate()
        refreshTimer = Timer.scheduledTimer(withTimeInterval: Timeout.autoRefresh, repeats: true) { [weak self] _ in
            guard let self else { return }
            Task { @MainActor in
                guard !self.replay.isActive else { return }
                await self.refetchAllSilent()
            }
        }
    }

    /// Set this tab's timeframe and refetch its charts.
    func setTimeRange(_ range: TimeRange) {
        if replay.isActive { chartViewModels.forEach { $0.clearGranularReplayData() } }
        selectedTimeRange = range
        markChanged()
        refetchAll()
        connectWebSocket()
    }

    /// Tell the CoinGecko service which coins are on screen, so it can prime all
    /// of their charts from one request instead of waiting out the per-chart queue.
    /// Must complete before the charts fetch — the list is what the prime covers.
    private func syncCoinGeckoSymbols() async {
        guard let cg = DataSourceFactory.shared.service(for: .coingecko) as? CoinGeckoAPIService else { return }
        let symbols = chartViewModels
            .filter { $0.source == .coingecko }
            .map { $0.apiSymbol }
        await cg.setActiveSymbols(symbols, for: tabID)
    }

    /// Open each provider's live stream for the symbols visible in this tab.
    private func connectWebSocket() {
        // A hidden tab has nothing to draw a tick onto.
        guard isWindowVisible, !replay.isActive else {
            wsService.disconnect()
            alpacaWSService.disconnect()
            return
        }

        let binanceVMs = marketChartViewModels.filter { $0.source == .binance }
        let symbols = binanceVMs.map { $0.apiSymbol.lowercased() }
        let interval = selectedTimeRange.binanceInterval

        if symbols.isEmpty {
            wsService.disconnect()
        } else {
            wsService.connect(symbols: symbols, interval: interval) { [weak self] symbol, kline in
                self?.chartViewModels
                    .first(where: { $0.source == .binance && $0.apiSymbol.uppercased() == symbol.uppercased() })?
                    .applyKlineUpdate(kline)
            }
        }

        let stockSymbols = marketChartViewModels.filter { $0.source == .alpaca }.map(\.apiSymbol)
        if stockSymbols.isEmpty || !AlpacaCredentialsStore.isConfigured {
            alpacaWSService.disconnect()
        } else {
            alpacaWSService.connect(symbols: stockSymbols) { [weak self] symbol, kline in
                guard let self else { return }
                self.chartViewModels
                    .first(where: { $0.source == .alpaca && $0.apiSymbol.uppercased() == symbol.uppercased() })?
                    .applyLiveBar(kline, candleDuration: self.selectedTimeRange.binanceIntervalSeconds)
            }
        }
    }

    /// Adjust candle count by a delta. Clamped to [min, max].
    /// Positive delta = zoom in (fewer candles), negative = zoom out (more candles).
    func adjustCandleCount(by delta: Int) {
        let newCount = (candleCount - delta).clamped(to: Candle.minCandles...Candle.maxCandles)
        guard newCount != candleCount else { return }
        candleCount = newCount
        markChanged()
        // Redraw from the warm-up headroom straight away; the refetch below only has
        // to top the buffer back up, so a zoom no longer waits on the network.
        chartViewModels.forEach { $0.setVisibleCount(newCount) }
        // Silent refetch — candle count change is visual feedback enough.
        refetchTask?.cancel()
        refetchTask = Task { [weak self] in
            guard let self else { return }
            await self.refetchAllVMs()
        }
    }

    private var refetchTask: Task<Void, Never>?

    /// Refetch with loading indicator (user-initiated: zoom, time range change).
    private func refetchAll() {
        refetchTask?.cancel()
        refetchTask = Task { [weak self] in
            guard let self else { return }
            self.isRefreshing = true
            defer { self.isRefreshing = false }
            await self.refetchAllVMs()
        }
    }

    /// Refetch without loading indicator (auto-refresh timer).
    private func refetchAllSilent() async {
        refetchTask?.cancel()
        let task = Task { [weak self] in
            guard let self else { return }
            await self.refetchAllVMs(silent: true)
        }
        refetchTask = task
        await task.value
    }

    private func refetchAllVMs(silent: Bool = false) async {
        await syncCoinGeckoSymbols()
        let range = self.selectedTimeRange
        let count = self.candleCount
        let fetches = self.chartViewModels.map { vm in
            Task { @MainActor in
                await vm.fetchData(for: range, count: count, silent: silent)
            }
        }
        await withTaskCancellationHandler {
            for fetch in fetches {
                await fetch.value
            }
        } onCancel: {
            fetches.forEach { $0.cancel() }
        }
        guard !Task.isCancelled else { return }
        if replay.isActive, replay.status != .selectingStart, let primary = marketChartViewModels.first {
            await prepareGranularReplay(interval: replay.session?.replayInterval ?? .automatic)
            replay.updateTimeline(primary.replayTimeline(), symbol: primary.apiSymbol, timeframe: range)
        }
    }

    func addPortfolioChart(_ config: PortfolioChartConfig) {
        let vm = ChartViewModel(
            ticker: "portfolio-\(UUID().uuidString)",
            source: .binance,
            displayName: config.kind.rawValue
        )
        vm.portfolioChart = config
        chartViewModels.append(vm)
        persistTickers()
        markChanged()
    }

    /// Add a ticker with a chosen data source.
    /// - Parameter displayName: label to show instead of the raw symbol, for sources
    ///   whose symbol is an opaque id (Polymarket CLOB token ids).
    /// - Parameter pmSeries: all tradable choices for multi-outcome Polymarket events.
    func addTicker(symbol: String, source: DataSourceType, displayName: String? = nil, pmSeries: [PmSeriesConfig]? = nil) async throws {
        // Duplicate check: same symbol + same source
        guard !chartViewModels.contains(where: {
            $0.ticker.uppercased() == symbol.uppercased() && $0.source == source
        }) else {
            throw TickerError.duplicate(displayName ?? symbol)
        }

        let vm = ChartViewModel(ticker: symbol, source: source, displayName: displayName)
        if let series = pmSeries, !series.isEmpty { vm.pmSeries = series }
        chartViewModels.append(vm)
        persistTickers()

        await syncCoinGeckoSymbols()
        await vm.fetchData(for: selectedTimeRange, count: candleCount)
        markChanged()
        connectWebSocket()
    }

    /// Remove a ticker and persist the change.
    func removeTicker(_ vm: ChartViewModel) {
        chartViewModels.removeAll { $0.uniqueID == vm.uniqueID }
        persistTickers()
        markChanged()
        connectWebSocket()
    }

    /// Load a favorite here when this tab has room; otherwise give it its own tab.
    func openFavorite(_ favorite: FavoriteItem) {
        if chartViewModels.count > 1 {
            WindowCoordinator.shared.newTab(for: favorite, beside: tabID)
            return
        }

        let config = favorite.config
        if let existing = marketChartViewModels.first {
            updateTicker(
                existing,
                symbol: config.symbol,
                source: config.source,
                displayName: config.displayName,
                pmSeries: config.pmSeries
            )
        } else {
            Task {
                try? await addTicker(
                    symbol: config.symbol,
                    source: config.source,
                    displayName: config.displayName,
                    pmSeries: config.pmSeries
                )
            }
        }
    }

    /// Update a chart's ticker symbol and/or source, then refetch.
    func updateTicker(_ vm: ChartViewModel, symbol: String, source: DataSourceType, displayName: String? = nil, pmSeries: [PmSeriesConfig]? = nil) {
        vm.updateTicker(symbol: symbol, source: source, displayName: displayName, pmSeries: pmSeries)
        persistTickers()
        markChanged()
        Task {
            await syncCoinGeckoSymbols()
            await vm.fetchData(for: selectedTimeRange, count: candleCount)
            if replay.isActive, let primary = marketChartViewModels.first {
                await prepareGranularReplay(interval: replay.session?.replayInterval ?? .automatic)
                replay.updateTimeline(primary.replayTimeline(), symbol: primary.apiSymbol, timeframe: selectedTimeRange)
            }
        }
        connectWebSocket()
    }

    /// Reorder tickers via drag-and-drop.
    func moveTicker(from source: IndexSet, to destination: Int) {
        chartViewModels.move(fromOffsets: source, toOffset: destination)
        persistTickers()
        markChanged()
    }

    private func makeTickerConfigs() -> [TickerConfig] {
        chartViewModels.map { vm in
            TickerConfig(
                symbol: vm.ticker,
                source: vm.source,
                bullishColorHex: vm.bullishColor.hexString,
                bearishColorHex: vm.bearishColor.hexString,
                yAxisDecimalPlaces: vm.yAxisDecimalPlaces,
                yZoom: vm.yZoom == 1 ? nil : vm.yZoom,
                showVolume: vm.showVolume ? true : nil,
                showRSI: vm.showRSI ? true : nil,
                showEMA: vm.showEMA ? true : nil,
                emaPeriod: vm.showEMA ? vm.emaPeriod : nil,
                showBollinger: vm.showBollinger ? true : nil,
                showTrendFlips: vm.showTrendFlips ? true : nil,
                // Drawings live in DrawingStore, keyed by source+ticker. Keep this
                // field nil so tabs and saved views no longer own copies of lines.
                trendLines: nil,
                displayName: vm.displayName,
                pmSeries: vm.pmSeries.isEmpty ? nil : vm.pmSeries,
                portfolioChart: vm.portfolioChart,
                pine: vm.pineConfiguration
            )
        }
    }

    private func persistTickers() {
        syncTab()
    }

    /// Write the whole tab record back. Cheap — `TabsStore` debounces the file write.
    private func syncTab() {
        guard !isHydrating else { return }
        let configs = makeTickerConfigs()
        TabsStore.shared.update(tabID) { tab in
            tab.name = tabName
            tab.savedViewID = currentViewID
            tab.tickerConfigs = configs
            tab.timeRange = selectedTimeRange
            tab.layoutMode = layoutMode
            tab.candleCount = candleCount
            tab.replaySession = replay.session
        }
    }

    // MARK: - Tab naming

    /// Rename the tab without touching the saved-view library.
    func renameTab(to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return }
        tabName = trimmed
        syncTab()
    }

    // MARK: - Saved Views

    /// Save the current state. Updates existing view if already named, otherwise creates new.
    func saveCurrentView(name: String) {
        let configs = makeTickerConfigs()
        let view = SavedView(
            id: currentViewID ?? UUID(),
            name: name,
            tickers: chartViewModels.map { $0.ticker },
            timeRange: selectedTimeRange,
            layoutMode: layoutMode,
            createdAt: Date(),
            tickerConfigs: configs,
            candleCount: candleCount
        )
        savedViews.removeAll { $0.id == view.id }
        savedViews.append(view)
        viewStore.save(savedViews)

        tabName = name
        currentViewID = view.id
        hasUnsavedChanges = false
        syncTab()
    }

    /// Save changes to the current named view without prompting.
    func saveChanges() {
        guard currentViewID != nil else {
            // No saved view yet — treat as new save; caller should prompt for name
            return
        }
        saveCurrentView(name: tabName)
    }

    /// Apply a saved view to this tab, replacing its current state.
    func loadView(_ view: SavedView) {
        replay.stop()
        isApplyingView = true
        defer { isApplyingView = false }

        chartViewModels.removeAll()
        selectedTimeRange = view.timeRange
        candleCount = view.candleCount ?? view.timeRange.dataPointLimit
        layoutMode = view.layoutMode

        let configs = view.resolvedConfigs
        chartViewModels = configs.map { config in
            let vm = ChartViewModel(ticker: config.symbol, source: config.source)
            vm.applyConfig(config)
            return vm
        }

        tabName = view.name
        currentViewID = view.id
        hasUnsavedChanges = false
        syncTab()

        didInitialLoad = true
        refetchAll()
        connectWebSocket()
    }

    /// Delete a saved view. Tabs sitting on it keep their charts but lose the link.
    func deleteView(_ view: SavedView) {
        savedViews.removeAll { $0.id == view.id }
        viewStore.save(savedViews)
        if currentViewID == view.id {
            tabName = UI.unnamedView
            currentViewID = nil
            hasUnsavedChanges = false
            syncTab()
        }
    }

    /// Persist chart appearance settings (colors, decimals) to disk.
    func persistChartSettings() {
        persistTickers()
        markChanged()
    }

    /// Mark current view as having unsaved changes (unless applying a loaded view).
    private func markChanged() {
        syncTab()
        guard !isApplyingView else { return }
        hasUnsavedChanges = true
    }

}

enum TickerError: LocalizedError {
    case duplicate(String)

    var errorDescription: String? {
        switch self {
        case .duplicate(let ticker):
            return "\"\(ticker)\" is already in your list"
        }
    }
}
