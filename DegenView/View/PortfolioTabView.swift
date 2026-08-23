import AppKit
import SwiftUI

/// Dedicated content for a portfolio tab. It deliberately has no chart toolbar,
/// ticker-add action, chart state, replay controls, or drawing-event monitors.
struct PortfolioTabView: View {
    let tabID: UUID
    @StateObject private var lifecycle: PortfolioTabLifecycle
    @StateObject private var store = PortfolioStore.shared
    @AppStorage("appTheme") private var appTheme: AppTheme = .system

    init(tabID: UUID) {
        self.tabID = tabID
        _lifecycle = StateObject(wrappedValue: PortfolioTabLifecycle(tabID: tabID))
    }

    var body: some View {
        PortfolioDashboardView(
            store: store,
            initialAsset: WindowCoordinator.shared.takeInitialPortfolioAsset(for: tabID),
            isTab: true
        )
        .navigationTitle("Portfolio")
        .background(
            WindowAccessor { window in lifecycle.attach(to: window) }
                .frame(width: 0, height: 0)
        )
        .preferredColorScheme(appTheme.colorScheme)
    }
}

@MainActor
private final class PortfolioTabLifecycle: ObservableObject {
    let tabID: UUID
    private weak var window: NSWindow?
    private var closeObserver: NSObjectProtocol?

    init(tabID: UUID) { self.tabID = tabID }

    func attach(to window: NSWindow) {
        guard self.window !== window else { return }
        self.window = window
        WindowCoordinator.shared.register(window, for: tabID)
        closeObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.willCloseNotification, object: window, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                WindowCoordinator.shared.unregister(self.tabID)
                self.removeObserver()
            }
        }
    }

    deinit {
        if let closeObserver { NotificationCenter.default.removeObserver(closeObserver) }
    }

    private func removeObserver() {
        if let closeObserver { NotificationCenter.default.removeObserver(closeObserver) }
        closeObserver = nil
    }
}
