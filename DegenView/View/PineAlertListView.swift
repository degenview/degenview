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
        AlertCardScroll {
            if !subscriptions.isEmpty {
                Section {
                    ForEach(subscriptions) { subscription in
                        row(subscription)
                    }
                } header: {
                    AlertDayHeader(title: "Alerts", count: subscriptions.count)
                }
            }
            ForEach(AlertDayGrouping.sections(history, date: \.triggeredAt)) { section in
                Section {
                    ForEach(section.items) { item in
                        recentRow(item)
                    }
                } header: {
                    AlertDayHeader(title: "Fired · \(section.title)", count: section.items.count)
                }
            }
        }
        .overlay {
            if store.subscriptions.isEmpty && store.history.isEmpty {
                ContentUnavailableView(
                    "No Script Alerts", systemImage: "curlybraces",
                    description: Text("Apply a script with alert() calls, then choose Create Alert in its settings."))
            } else if subscriptions.isEmpty && history.isEmpty {
                ContentUnavailableView.search(text: search)
            }
        }
    }

    private func row(_ subscription: PineAlertSubscription) -> some View {
        let status = status(of: subscription)
        let coordinator = PineAlertCoordinator.shared
        return PineAlertCard {
            scriptIcon(symbolKey: subscription.symbolKey)
            VStack(alignment: .leading, spacing: 2) {
                Text(subscription.scriptName).font(.headline).lineLimit(1)
                Text(
                    subscription.note.isEmpty
                        ? "\(subscription.symbolKey.symbolPart) · \(subscription.timeframe)" : subscription.note
                )
                .font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 8)
            SettingsStatusBadge(text: status.text, tone: status.tone)
            Menu {
                if subscription.isActive {
                    Button("Pause", systemImage: "pause.fill") { coordinator.pause(subscription: subscription.id) }
                } else {
                    Button("Re-arm", systemImage: "play.fill") { coordinator.rearm(subscription: subscription.id) }
                        .disabled(!coordinator.canRearm(subscription))
                }
                Divider()
                Button("Delete", systemImage: "trash", role: .destructive) {
                    coordinator.remove(subscription: subscription.id)
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 13, weight: .semibold))
                    .frame(width: 26, height: 26)
                    .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("More actions")
        }
    }

    private func recentRow(_ item: PineAlertNotification) -> some View {
        PineAlertCard {
            scriptIcon(symbolKey: item.symbolKey)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title).font(.headline).lineLimit(1)
                Text(item.message.isEmpty ? "Alert" : item.message)
                    .font(.caption).foregroundStyle(.secondary).lineLimit(2)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 2) {
                Text(item.triggeredAt.formatted(date: .omitted, time: .shortened)).monospacedDigit()
                Text(item.triggeredAt.formatted(.relative(presentation: .numeric, unitsStyle: .abbreviated)))
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    /// A script glyph with the market's data source badged on the corner.
    private func scriptIcon(symbolKey: String) -> some View {
        let size: CGFloat = 34
        let badge = (size * 0.55).rounded()
        return Image(systemName: "curlybraces")
            .font(.system(size: size * 0.42, weight: .semibold))
            .foregroundStyle(.tint)
            .frame(width: size, height: size)
            .background(Color.accentColor.opacity(0.14), in: Circle())
            .overlay(alignment: .bottomTrailing) {
                if let source = symbolKey.source {
                    SourceLogoView(source: source, size: badge)
                        .padding(1)
                        .background(Color(nsColor: .windowBackgroundColor), in: RoundedRectangle(cornerRadius: badge * 0.28))
                        .offset(x: badge * 0.3, y: badge * 0.3)
                }
            }
            .padding(.trailing, 3)
            .padding(.bottom, 3)
            .accessibilityHidden(true)
    }

    private func status(of subscription: PineAlertSubscription) -> (text: String, tone: SettingsStatusBadge.Tone) {
        switch subscription.state {
        case .paused: return ("Paused", .neutral)
        case .scriptChanged: return ("Script changed — re-arm", .warning)
        case .scriptDeleted: return ("Script deleted", .bad)
        case .active:
            guard let dataset = store.chartDatasets[subscription.chartID] else { return ("Chart not open", .neutral) }
            return subscription.watches(dataset) ? ("Active", .good) : ("Other symbol on chart", .warning)
        }
    }

    private func matches(_ text: String) -> Bool {
        search.isEmpty || text.localizedCaseInsensitiveContains(search)
    }
}

extension String {
    /// For a `"<source>:<ticker>"` key, the data source it names.
    fileprivate var source: DataSourceType? {
        split(separator: ":", maxSplits: 1).first.flatMap { DataSourceType(rawValue: String($0)) }
    }

    /// For a `"<source>:<ticker>"` key, the ticker.
    fileprivate var symbolPart: String {
        split(separator: ":", maxSplits: 1).last.map(String.init) ?? self
    }
}
