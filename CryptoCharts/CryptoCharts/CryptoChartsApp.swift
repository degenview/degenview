import SwiftUI
import AppKit

@main
struct CryptoChartsApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
                .onReceive(
                    NotificationCenter.default.publisher(for: NSApplication.willTerminateNotification)
                ) { _ in
                    flushCaches()
                }
        }
        .defaultSize(width: 440, height: 700)
        .windowResizability(.contentMinSize)
    }

    /// Persist rate-limited sources' candles on quit so the next launch draws
    /// from disk instead of the request queue.
    private func flushCaches() {
        guard let cg = DataSourceFactory.shared.service(for: .coingecko) as? CoinGeckoAPIService else { return }
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
