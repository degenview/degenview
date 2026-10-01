import SwiftUI

/// The Script Alerts section of the alerts center: subscriptions to Pine `alert()` calls, and what
/// they delivered.
struct PineAlertListView: View {
    let search: String
    @StateObject private var store = PineAlertStore.shared

    private var subscriptions: [PineAlertSubscription] {
        store.subscriptions.filter { matches($0.scriptName) || matches($0.symbolKey) }
            .sorted { $0.createdAt > $1.createdAt }
    }

    private var history: [PineAlertNotification] {
        store.history.filter { matches($0.scriptName) || matches($0.symbolKey) || matches($0.message) }
    }

    var body: some View {
        List {
            Section("Alerts") {
                ForEach(subscriptions) { subscription in
                    row(subscription)
                }
            }
            Section("Recent") {
                ForEach(history) { item in
                    HStack(alignment: .firstTextBaseline) {
                        VStack(alignment: .leading) {
                            Text(item.title).font(.headline)
                            Text(item.message.isEmpty ? "Alert" : item.message).font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Text(item.triggeredAt, style: .relative).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .overlay {
            if store.subscriptions.isEmpty && store.history.isEmpty {
                ContentUnavailableView(
                    "No Script Alerts", systemImage: "chevron.left.forwardslash.chevron.right",
                    description: Text("Apply a script with alert() calls, then choose Create Alert in its settings."))
            }
        }
    }

    private func row(_ subscription: PineAlertSubscription) -> some View {
        let status = status(of: subscription)
        let coordinator = PineAlertCoordinator.shared
        return HStack {
            VStack(alignment: .leading) {
                Text(subscription.scriptName).font(.headline)
                Text(subscription.note.isEmpty ? "\(subscription.symbolKey) · \(subscription.timeframe)" : subscription.note)
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Text(status.text).font(.caption).foregroundStyle(status.color)
            Menu {
                if subscription.isActive {
                    Button("Pause") { coordinator.pause(subscription: subscription.id) }
                } else {
                    Button("Re-arm") { coordinator.rearm(subscription: subscription.id) }
                        .disabled(!coordinator.canRearm(subscription))
                }
                Divider()
                Button("Delete", role: .destructive) { coordinator.remove(subscription: subscription.id) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
        }
        .padding(.vertical, 5)
    }

    private func status(of subscription: PineAlertSubscription) -> (text: String, color: Color) {
        switch subscription.state {
        case .paused: return ("Paused", .secondary)
        case .scriptChanged: return ("Script changed — re-arm to resume", .orange)
        case .scriptDeleted: return ("Script deleted", .red)
        case .active:
            guard let dataset = store.chartDatasets[subscription.chartID] else { return ("Chart not open", .secondary) }
            return subscription.watches(dataset) ? ("Active", .green) : ("Chart shows other symbol", .orange)
        }
    }

    private func matches(_ text: String) -> Bool {
        search.isEmpty || text.localizedCaseInsensitiveContains(search)
    }
}
