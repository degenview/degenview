import AppKit
import SwiftUI

/// The AppKit seam for tabs.
///
/// Tabs are real `NSWindow`s in a tab group, so reordering, dragging a tab out
/// into its own window, and dragging a window back onto a tab bar are all done
/// by AppKit — none of that is implemented here. This type only has to
/// (a) put newly opened windows into the right tab group and (b) read the
/// resulting arrangement back out so it survives relaunch.
@MainActor
final class WindowCoordinator {
    static let shared = WindowCoordinator()

    /// Shared by every window, so AppKit treats them as one tabbing family.
    private static let tabbingIdentifier = "com.cryptocharts.charts"

    private final class WeakWindow {
        weak var window: NSWindow?
        init(_ window: NSWindow) { self.window = window }
    }

    private var windows: [UUID: WeakWindow] = [:]

    /// Tab a freshly opened window should join, recorded before `openWindow`
    /// steals key status from the window we want to anchor on.
    private var pendingJoins: [UUID: UUID] = [:]
    private var pendingPortfolioAssets: [UUID: PortfolioAsset] = [:]
    private weak var pendingAuxiliaryAnchor: NSWindow?

    /// Whether the launch window has already claimed the persisted session.
    /// Everything opened afterwards without a tab id — the tab bar's `+`,
    /// File ▸ New Window — is a *new* tab, not a second view onto an old one.
    private var hasAdoptedSession = false

    /// Closing a window normally deletes its tab. During termination the windows
    /// close too, and that must not wipe the snapshot we are about to write.
    private var isTerminating = false

    private init() {
        // Detaching a tab and merging windows both leave a group at one window,
        // where AppKit drops the tab bar. Neither posts anything more specific
        // than a window becoming key.
        NotificationCenter.default.addObserver(
            forName: NSWindow.didBecomeKeyNotification,
            object: nil,
            queue: .main
        ) { _ in
            Task { @MainActor in WindowCoordinator.shared.refreshTabBars() }
        }
    }

    // MARK: - Lookup

    func window(for tabID: UUID) -> NSWindow? { windows[tabID]?.window }

    private func tabID(for window: NSWindow) -> UUID? {
        windows.first { $0.value.window === window }?.key
    }

    /// The tab the user is currently looking at — the anchor for a new tab.
    private func frontmostTabID() -> UUID? {
        if let key = NSApp.keyWindow, let id = tabID(for: key) { return id }
        if let main = NSApp.mainWindow, let id = tabID(for: main) { return id }
        return nil
    }

    // MARK: - Registration

    /// Capture the current chart window before opening an auxiliary SwiftUI scene,
    /// because the new scene may become key before its `WindowAccessor` resolves.
    func prepareAuxiliaryTab() {
        if let id = frontmostTabID() { pendingAuxiliaryAnchor = window(for: id) }
    }

    /// Join a manager/editor-style scene to the native chart tab group without
    /// registering it as a persisted chart tab.
    func registerAuxiliaryTab(_ window: NSWindow) {
        window.tabbingIdentifier = Self.tabbingIdentifier
        window.tabbingMode = .preferred
        window.isRestorable = false
        let anchor =
            pendingAuxiliaryAnchor
            ?? NSApp.windows.first(where: { tabID(for: $0) != nil && $0 !== window })
        pendingAuxiliaryAnchor = nil
        if let anchor, anchor.tabGroup?.windows.contains(window) != true {
            anchor.addTabbedWindow(window, ordered: .above)
            window.makeKeyAndOrderFront(nil)
        }
        ensureTabBarVisible(window)
    }

    func register(_ window: NSWindow, for tabID: UUID) {
        window.tabbingIdentifier = Self.tabbingIdentifier
        window.tabbingMode = .preferred
        // We rebuild windows from tabs.json ourselves; AppKit's own restoration
        // would reopen a second set on top.
        window.isRestorable = false
        windows[tabID] = WeakWindow(window)

        var joinedTabGroup = false
        if let anchorID = pendingJoins.removeValue(forKey: tabID),
            let anchor = self.window(for: anchorID),
            anchor !== window,
            anchor.tabGroup?.windows.contains(window) != true
        {
            anchor.addTabbedWindow(window, ordered: .above)
            window.makeKeyAndOrderFront(nil)
            joinedTabGroup = true
        }

        // A tab takes the frame of the group it joined; only a window standing
        // on its own has a frame to decide.
        if !joinedTabGroup { applyStartingFrame(to: window) }

        syncTitle(window, tabID: tabID)

        // The bar carries the `+` button and is the only place a lone tab can be
        // renamed or dragged from, so it stays up even at one tab.
        ensureTabBarVisible(window)
    }

    /// Whether the window whose frame is remembered has been placed yet.
    private var didPlaceInitialWindow = false

    /// Size, place, and then track the first standalone window of the session.
    ///
    /// It reopens where the user last left it; failing that — cold install, or a
    /// frame saved on a display that is no longer attached — it gets a landscape
    /// frame scaled to the screen it opened on.
    ///
    /// The frame is written by hand rather than through `setFrameAutosaveName`:
    /// SwiftUI keeps reasserting its own autosave name on the window, and that
    /// name embeds the scene's load address, so it changes with every build and
    /// nothing saved under it is ever read back.
    private func applyStartingFrame(to window: NSWindow) {
        guard !didPlaceInitialWindow else { return }
        didPlaceInitialWindow = true

        let screens = NSScreen.screens.map(\.visibleFrame)
        if let saved = savedFrame(), WindowFrame.isReachable(saved, on: screens) {
            window.setFrame(saved, display: false)
        } else if let visible = (window.screen ?? NSScreen.main)?.visibleFrame {
            window.setFrame(WindowFrame.default(in: visible), display: false)
        }

        for name in [NSWindow.didEndLiveResizeNotification, NSWindow.didMoveNotification] {
            NotificationCenter.default.addObserver(
                forName: name, object: window, queue: .main
            ) { [weak window] _ in
                guard let window else { return }
                Task { @MainActor in WindowCoordinator.shared.rememberFrame(of: window) }
            }
        }
    }

    /// A full-screen or zoomed window is a temporary state, not the size the user
    /// wants the app to open at.
    private func rememberFrame(of window: NSWindow) {
        guard !window.styleMask.contains(.fullScreen), !window.isZoomed else { return }
        UserDefaults.standard.set(
            NSStringFromRect(window.frame),
            forKey: UI.windowFrameDefaultsKey)
    }

    private func savedFrame() -> NSRect? {
        guard let raw = UserDefaults.standard.string(forKey: UI.windowFrameDefaultsKey)
        else { return nil }
        let frame = NSRectFromString(raw)
        // Guards a truncated or hand-edited value, which parses as zero.
        guard frame.width >= UI.windowMinWidth, frame.height >= UI.windowMinHeight
        else { return nil }
        return frame
    }

    /// AppKit hides the bar whenever a group is down to one window. Re-asserting
    /// it only sticks because `AppDelegate` answers `newWindowForTab:` — without
    /// that responder AppKit reports the bar visible and still refuses to draw it.
    private func ensureTabBarVisible(_ window: NSWindow) {
        guard window.tabGroup?.isTabBarVisible != true else { return }
        window.toggleTabBar(nil)
    }

    /// Detaching and merging both leave a group at one window, so the bar has to
    /// be re-asserted across every window, not set once.
    private func refreshTabBars() {
        for box in windows.values {
            guard let window = box.window, window.isVisible else { continue }
            ensureTabBarVisible(window)
        }
    }

    /// Pull every window back into one tab group.
    func mergeAllWindows() {
        NSApp.sendAction(#selector(NSWindow.mergeAllWindows(_:)), to: nil, from: nil)
        // The merge lands after the current run loop pass.
        Task { @MainActor in
            self.refreshTabBars()
            if let key = NSApp.keyWindow { self.ensureTabBarVisible(key) }
        }
    }

    func unregister(_ tabID: UUID) {
        windows[tabID] = nil
        guard !isTerminating else { return }
        TabsStore.shared.removeTab(tabID)
        // Closing a tab can leave the group at one window, which drops the bar.
        refreshTabBars()
    }

    /// Keep the tab label in step with the tab name — on macOS the tab title
    /// *is* the window title.
    func syncTitle(_ window: NSWindow, tabID: UUID) {
        let name = TabsStore.shared.tab(tabID)?.name ?? UI.unnamedView
        if window.title != name { window.title = name }
    }

    func syncTitle(for tabID: UUID) {
        guard let window = window(for: tabID) else { return }
        syncTitle(window, tabID: tabID)
    }

    // MARK: - Opening tabs

    /// Resolve the tab for a window SwiftUI opened *without* a value.
    ///
    /// Only the launch window adopts the persisted session. The tab bar's `+`
    /// and File ▸ New Window also arrive here with no value, and they must mint
    /// a blank tab — otherwise they open a second window onto a tab that is
    /// already on screen.
    /// - Returns: the tab id, and whether this window is the one that should
    ///   reopen the rest of the session.
    func tabForUnvaluedWindow() -> (id: UUID, adoptedSession: Bool) {
        guard hasAdoptedSession else {
            hasAdoptedSession = true
            return (TabsStore.shared.firstTabID(), true)
        }

        let anchor = frontmostTabID()
        let tab = TabsStore.shared.makeTab()
        if let anchor { pendingJoins[tab.id] = anchor }
        return (tab.id, false)
    }

    /// Captured from a window's environment so the `+` button and the app
    /// delegate — neither of which sits in a SwiftUI view — can open windows.
    private var openWindowAction: OpenWindowAction?

    func useOpenWindowAction(_ action: OpenWindowAction) {
        guard openWindowAction == nil else { return }
        openWindowAction = action
    }

    /// Open a new empty tab next to the one the user is on.
    func newTab(using action: OpenWindowAction? = nil) {
        guard let openWindow = action ?? openWindowAction else { return }
        let anchor = frontmostTabID()
        let tab = TabsStore.shared.makeTab()
        if let anchor { pendingJoins[tab.id] = anchor }
        openWindow(value: tab.id)
    }

    /// Open a favorite as a new system tab beside the tab that requested it.
    func newTab(for favorite: FavoriteItem, beside anchorID: UUID) {
        guard let openWindow = openWindowAction else { return }
        let tab = TabsStore.shared.makeTab(name: favorite.name, tickerConfig: favorite.config)
        pendingJoins[tab.id] = anchorID
        openWindow(value: tab.id)
    }

    /// Opens the single portfolio workspace as a real AppKit tab. Repeated requests
    /// focus the existing tab instead of creating duplicate portfolio workspaces.
    func openPortfolio(beside anchorID: UUID, initialAsset: PortfolioAsset? = nil) {
        guard let openWindow = openWindowAction else { return }
        let tab = TabsStore.shared.portfolioTab ?? TabsStore.shared.makePortfolioTab()
        if let initialAsset { pendingPortfolioAssets[tab.id] = initialAsset }

        if let existing = window(for: tab.id) {
            existing.makeKeyAndOrderFront(nil)
            if let initialAsset {
                NotificationCenter.default.post(name: .portfolioAddTransaction, object: initialAsset)
                pendingPortfolioAssets[tab.id] = nil
            }
            return
        }

        pendingJoins[tab.id] = anchorID
        openWindow(value: tab.id)
    }

    func takeInitialPortfolioAsset(for tabID: UUID) -> PortfolioAsset? {
        pendingPortfolioAssets.removeValue(forKey: tabID)
    }

    // MARK: - Restore

    private var didRestore = false

    /// Reopen every persisted tab except the one the first window already
    /// adopted, re-tabbing each into the window group it belonged to at quit.
    func restoreWindows(adopted: UUID, using openWindow: OpenWindowAction) async {
        guard !didRestore else { return }
        didRestore = true

        // The tabs that follow anchor on this window, so it has to be on screen
        // and registered before any of them open.
        await awaitRegistration(of: adopted)

        let store = TabsStore.shared
        for id in store.remainingTabIDs(excluding: adopted) {
            // Anchor on whichever member of this tab's window group is already
            // on screen. None means it was a standalone window — leave it loose.
            if let groupIndex = store.windowIndex(of: id),
                let anchor = store.windowGroups[groupIndex].first(where: { window(for: $0) != nil })
            {
                pendingJoins[id] = anchor
            }
            openWindow(value: id)
            await awaitRegistration(of: id)
        }

        // The adopted window ends up behind the ones opened after it.
        window(for: adopted)?.makeKeyAndOrderFront(nil)
    }

    /// The next tab may need to anchor on this one, so it has to exist first.
    private func awaitRegistration(of tabID: UUID) async {
        for _ in 0..<40 {  // ~2 s ceiling
            if window(for: tabID) != nil { return }
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
    }

    // MARK: - Capture

    /// Read the current arrangement out of AppKit and hand it to `TabsStore`.
    ///
    /// The user does all the reordering, detaching, and merging through the
    /// system tab bar, so `NSWindow.tabGroup` is the only authority on layout —
    /// there is nothing to track incrementally.
    func captureGrouping() {
        var groups: [[UUID]] = []
        var visited = Set<ObjectIdentifier>()

        for window in NSApp.windows {
            guard tabID(for: window) != nil,
                visited.insert(ObjectIdentifier(window)).inserted
            else { continue }

            let members = window.tabGroup?.windows ?? [window]
            members.forEach { visited.insert(ObjectIdentifier($0)) }

            let ids = members.compactMap { tabID(for: $0) }
            if !ids.isEmpty { groups.append(ids) }
        }

        // Put the group the user was last in first, so the next launch adopts a
        // tab from it into the initial window.
        if let main = NSApp.mainWindow ?? NSApp.keyWindow,
            let mainID = tabID(for: main),
            let index = groups.firstIndex(where: { $0.contains(mainID) }),
            index != 0
        {
            groups.swapAt(0, index)
        }

        TabsStore.shared.setWindowGroups(groups)
    }

    func applicationWillTerminate() {
        isTerminating = true
        captureGrouping()
        TabsStore.shared.persist()
    }
}

extension Notification.Name {
    static let portfolioAddTransaction = Notification.Name("portfolioAddTransaction")
}
