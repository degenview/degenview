import AppKit
import SwiftUI
import UserNotifications

@main
struct DegenViewApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        // Value-based so each window carries the id of the tab it renders.
        // Detaching and merging tabs moves whole windows, so a tab's charts,
        // fetches, and WebSocket never migrate between scenes.
        WindowGroup(for: UUID.self) { $tabID in
            ChartTabRoot(requestedID: tabID)
        }
        // A landscape first guess, so the screen-fitted frame `WindowCoordinator`
        // applies once the window is on screen isn't a visible jump.
        .defaultSize(width: UI.windowIdealWidth, height: UI.windowIdealHeight)
        .windowResizability(.contentMinSize)
        .commands { TabCommands() }

        Settings {
            AppSettingsView()
        }

        Window("Alerts", id: "alerts") {
            AlertsCenterView()
        }
        .defaultSize(width: 880, height: 620)

        Window("Script Manager", id: "script-manager") { ScriptManagerView() }
            .defaultSize(width: 900, height: 600)
    }
}

// MARK: - Commands

private struct TabCommands: Commands {
    @Environment(\.openWindow) private var openWindow
    @FocusedValue(\.savedLayout) private var savedLayout

    var body: some Commands {
        CommandGroup(replacing: .saveItem) {
            Button("Save Layout") { savedLayout?.requestSave() }
                .keyboardShortcut("s", modifiers: .command)
                .disabled(savedLayout == nil)
        }
        CommandGroup(after: .newItem) {
            Button("New Tab") {
                WindowCoordinator.shared.newTab(using: openWindow)
            }
            .keyboardShortcut("t", modifiers: .command)

            Button("Merge All Windows") {
                WindowCoordinator.shared.mergeAllWindows()
            }
        }
        CommandGroup(after: .undoRedo) {
            Button("Redo") {
                NSApp.sendAction(Selector(("redo:")), to: nil, from: nil)
            }
            .keyboardShortcut("y", modifiers: .command)
        }
        CommandGroup(after: .windowArrangement) {
            Button("Script Manager") {
                WindowCoordinator.shared.prepareAuxiliaryTab()
                openWindow(id: "script-manager")
            }
        }
    }
}

// MARK: - Window root

/// Resolves which tab this window renders.
///
/// SwiftUI hands the very first window a nil value; that window adopts the
/// first persisted tab and then reopens the rest, re-tabbing each into the
/// window group it belonged to at quit.
private struct ChartTabRoot: View {
    @Environment(\.openWindow) private var openWindow

    /// Resolved up front rather than in `.task`: a first frame without a
    /// `ContentView` publishes no `navigationTitle`, and SwiftUI keeps the
    /// default window title it picked then — which is the tab's label.
    @StateObject private var tab: ResolvedTab

    init(requestedID: UUID?) {
        _tab = StateObject(wrappedValue: ResolvedTab(requested: requestedID))
    }

    var body: some View {
        Group {
            if TabsStore.shared.tab(tab.id)?.kind == .portfolio {
                PortfolioTabView(tabID: tab.id)
            } else {
                ContentView(tabID: tab.id)
            }
        }
        .overlay(alignment: .top) { GlobalAlertBanner() }
        .task {
            // The tab bar's + button reaches the app delegate, which has no
            // SwiftUI environment of its own to open windows from.
            WindowCoordinator.shared.useOpenWindowAction(openWindow)
            guard tab.adoptedSession else { return }
            await WindowCoordinator.shared.restoreWindows(adopted: tab.id, using: openWindow)
        }
    }
}

/// Which tab a window renders, decided once per window.
///
/// This has to be a `StateObject`: resolving a nil scene value can *create* a
/// tab, and a plain `State(initialValue:)` would re-run that side effect on
/// every re-init of the enclosing view even though only the first value is kept.
private final class ResolvedTab: ObservableObject {
    let id: UUID
    /// True only for the launch window, which reopens the rest of the session.
    let adoptedSession: Bool

    init(requested: UUID?) {
        if let requested {
            id = requested
            adoptedSession = false
        } else {
            let resolved = MainActor.assumeIsolated {
                WindowCoordinator.shared.tabForUnvaluedWindow()
            }
            id = resolved.id
            adoptedSession = resolved.adoptedSession
        }
    }
}

// MARK: - App delegate

final class AppDelegate: NSObject, NSApplicationDelegate {

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSWindow.allowsAutomaticWindowTabbing = true
        UNUserNotificationCenter.current().delegate = AlertNotificationDelegate.shared
        _ = AlertStore.shared
        _ = PineAlertCoordinator.shared
        NotificationCenter.default.addObserver(
            forName: .localScriptsDidChange, object: nil, queue: .main
        ) { _ in
            Task { @MainActor in await PineAlertCoordinator.shared.scriptsChanged() }
        }
        ScriptFolderMonitor.shared.start()
        // A test run is hosted by the app and must not write into the real script library.
        if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil {
            Task { await DemoScriptLibrary.seedIfNeeded() }
        }
    }

    /// `.pine` files opened from Finder. Files already in the script library open straight
    /// into the Script Manager; any other file is offered for import (copied, never moved).
    func application(_ application: NSApplication, open urls: [URL]) {
        let scripts = urls.filter { $0.pathExtension.caseInsensitiveCompare(ScriptStore.fileExtension) == .orderedSame }
        Task { @MainActor in
            for url in scripts { await openScriptFile(url) }
        }
    }

    @MainActor private func openScriptFile(_ url: URL) async {
        do {
            if let id = try await ScriptStore.shared.scriptID(forFileAt: url) {
                WindowCoordinator.shared.openScriptManager(selecting: id)
                return
            }
            let alert = NSAlert()
            alert.messageText = "Import “\(url.lastPathComponent)”?"
            alert.informativeText =
                "DegenView keeps scripts in its Scripts folder. The file will be copied there and opened in the Script Manager; the original stays where it is."
            alert.addButton(withTitle: "Import")
            alert.addButton(withTitle: "Cancel")
            NSApp.activate(ignoringOtherApps: true)
            guard alert.runModal() == .alertFirstButtonReturn else { return }
            let script = try await ScriptStore.shared.importFile(at: url)
            NotificationCenter.default.post(name: .localScriptsDidChange, object: script.id)
            WindowCoordinator.shared.openScriptManager(selecting: script.id)
        } catch {
            let alert = NSAlert(error: error)
            alert.messageText = "Couldn't open “\(url.lastPathComponent)”"
            alert.informativeText = error.localizedDescription
            alert.runModal()
        }
    }

    /// The tab bar's `+` button.
    ///
    /// This also has a side effect worth keeping: AppKit only draws the tab bar
    /// for a *single*-tab window when something in the responder chain answers
    /// `newWindowForTab:`. Without it the bar appears only at two tabs or more,
    /// no matter what `toggleTabBar` reports.
    @IBAction func newWindowForTab(_ sender: Any?) {
        MainActor.assumeIsolated {
            WindowCoordinator.shared.newTab()
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        MainActor.assumeIsolated {
            // Read the tab arrangement back out of AppKit before the windows go.
            WindowCoordinator.shared.applicationWillTerminate()
        }
        flushCaches()
    }

    /// Persist rate-limited sources' candles on quit so the next launch draws
    /// from disk instead of the request queue.
    private func flushCaches() {
        guard
            let cg = MainActor.assumeIsolated({
                DataSourceFactory.shared.service(for: .coingecko) as? CoinGeckoAPIService
            })
        else { return }
        // willTerminate gives us the run loop, not an async context — block
        // briefly so the write lands before the process goes away.
        let done = DispatchSemaphore(value: 0)
        Task {
            await cg.flushCache()
            done.signal()
        }
        _ = done.wait(timeout: .now() + 1.0)
    }
}

final class AlertNotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    static let shared = AlertNotificationDelegate()
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async
        -> UNNotificationPresentationOptions
    {
        let settings = await MainActor.run { AlertStore.shared.settings }
        guard settings.deliveryEnabled && settings.macOSNotificationsEnabled else { return [] }
        return settings.soundEnabled ? [.banner, .sound] : [.banner]
    }
    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        await MainActor.run { NSApp.activate(ignoringOtherApps: true) }
    }
}
