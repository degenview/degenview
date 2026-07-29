import SwiftUI

@main
struct CryptoChartsApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
        .defaultSize(width: 440, height: 700)
        .windowResizability(.contentMinSize)
    }
}
