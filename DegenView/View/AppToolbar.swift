import SwiftUI

/// Title-bar buttons every tab kind carries — chart, portfolio and Script Manager.
///
/// Both actions go through `WindowCoordinator`, so the buttons need no tab-specific state
/// and keep working from the Script Manager scene, which has no tab id of its own.
struct AppToolbar: ToolbarContent {
    var body: some ToolbarContent {
        ToolbarItemGroup(placement: .automatic) {
            Button {
                WindowCoordinator.shared.openPortfolio()
            } label: {
                Label("Portfolio", systemImage: "briefcase")
            }
            .help("Portfolio Tracker")

            Button {
                WindowCoordinator.shared.openScriptManager()
            } label: {
                Label("Script Manager", systemImage: "curlybraces")
            }
            .help("Script Manager")
        }
    }
}
