import SwiftUI

/// The Paper Trading panel under the charts: account header and tabs, the account's numbers, one
/// table per section, and a footer with the section's bulk action.
struct PaperAccountManagerView: View {
    @ObservedObject var store: PaperTradingStore
    @Binding var selectedTab: PaperManagerTab
    @Binding var showTradingOnCharts: Bool
    let onClose: () -> Void
    @State private var showCreate = false
    @State private var showReset = false

    var body: some View {
        VStack(spacing: 0) {
            PaperPanelHeader(
                store: store, selectedTab: $selectedTab, showTradingOnCharts: $showTradingOnCharts,
                onCreate: { showCreate = true }, onExport: exportCSV,
                onReset: { showReset = true }, onClose: onClose)
            Divider()
            if store.selectedAccount == nil {
                PaperEmptyState(
                    systemImage: "doc.text", title: "No paper account",
                    message: "Create one to practise trading with simulated money.",
                    actionTitle: "Create Account"
                ) {
                    showCreate = true
                }
            } else {
                PaperMetricsStrip(metrics: store.metrics, currency: store.accountCurrency)
                Divider()
                content.frame(maxWidth: .infinity, maxHeight: .infinity)
                PaperPanelFooter(store: store, tab: selectedTab, onExport: exportCSV)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .sheet(isPresented: $showCreate) {
            PaperAccountConfigurationSheet(mode: .create) { name, currency, balance, settings in
                Task { await store.createAccount(name: name, currency: currency, balance: balance, settings: settings) }
            }
        }
        .sheet(isPresented: $showReset) {
            if let account = store.selectedAccount {
                PaperAccountConfigurationSheet(mode: .reset(account)) { _, currency, balance, settings in
                    Task { await store.reset(currency: currency, balance: balance, settings: settings) }
                }
            }
        }
    }

    @ViewBuilder private var content: some View {
        switch selectedTab {
        case .positions: PaperPositionsTable(store: store)
        case .orders: PaperOrdersTable(store: store)
        case .history: PaperOrderHistoryTable(store: store)
        case .accountHistory: PaperClosedTradesTable(store: store)
        case .journal: PaperJournalTable(store: store)
        }
    }

    private func exportCSV() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Export"
        guard panel.runModal() == .OK, let directory = panel.url else { return }
        PaperTradingCSVExporter.export(snapshot: store.snapshot, accountID: store.selectedAccount?.id, to: directory)
    }
}
