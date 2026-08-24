import SwiftUI

struct AlertsCenterView: View {
    enum Filter: String, CaseIterable { case active = "Active", triggered = "Triggered", paused = "Paused", all = "All", history = "History" }
    @StateObject private var store = AlertStore.shared
    @State private var filter: Filter = .active
    @State private var search = ""
    @State private var editing: PriceAlert?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker("View", selection: $filter) { ForEach(Filter.allCases, id: \.self) { Text($0.rawValue).tag($0) } }.pickerStyle(.segmented)
                TextField("Search assets", text: $search).textFieldStyle(.roundedBorder).frame(width: 190)
            }.padding()
            Divider()
            if filter == .history { historyList } else { alertList }
            Divider()
            HStack { Label("Local only · Evaluates while DegenView is running", systemImage: "desktopcomputer").font(.caption).foregroundStyle(.secondary); Spacer(); if filter == .history { Button("Clear History") { Task { await store.clearHistory() } } } }.padding(10)
        }
        .frame(minWidth: 720, minHeight: 430)
        .sheet(item: $editing) { PriceAlertEditor(asset: $0.asset, existing: $0) }
    }
    private var filtered: [PriceAlert] { store.alerts.filter { alert in
        let stateOK = filter == .all || (filter == .active && alert.state == .active) || (filter == .paused && alert.state == .paused) || (filter == .triggered && alert.state == .triggered)
        return stateOK && (search.isEmpty || alert.asset.symbol.localizedCaseInsensitiveContains(search) || alert.asset.name.localizedCaseInsensitiveContains(search))
    }.sorted { $0.updatedAt > $1.updatedAt } }
    private var alertList: some View {
        List(filtered) { alert in
            HStack {
                VStack(alignment: .leading) { Text(alert.asset.symbol).font(.headline); Text(alert.note.isEmpty ? alert.asset.source.displayName : alert.note).font(.caption).foregroundStyle(.secondary) }
                Spacer()
                VStack(alignment: .trailing) { Text("\(alert.condition.target?.description ?? "Unsupported") \(alert.currency.rawValue)").monospacedDigit(); Text(status(alert)).font(.caption).foregroundStyle(.secondary) }
                Menu { Button("Edit") { editing = alert }; if alert.state == .active { Button("Pause") { Task { await store.pause(alert.id) } } } else { Button("Resume") { Task { await store.resume(alert.id) } } }; Button("Duplicate") { var copy = alert; copy = PriceAlert(asset: copy.asset, condition: copy.condition, currency: copy.currency, frequency: copy.frequency, note: copy.note); Task { await store.save(copy) } }; Divider(); Button("Delete", role: .destructive) { Task { await store.delete(alert.id) } } } label: { Image(systemName: "ellipsis.circle") }
            }.padding(.vertical, 5)
        }.overlay { if filtered.isEmpty { ContentUnavailableView("No Alerts", systemImage: "bell") } }
    }
    private var historyList: some View { List(store.history.filter { search.isEmpty || $0.asset.symbol.localizedCaseInsensitiveContains(search) }) { event in HStack { Text(event.asset.symbol).font(.headline); Spacer(); Text("\(event.observedValue) \(event.currency.rawValue)").monospacedDigit(); Text(event.timestamp, style: .relative).foregroundStyle(.secondary) } }.overlay { if store.history.isEmpty { ContentUnavailableView("No Trigger History", systemImage: "clock") } } }
    private func status(_ alert: PriceAlert) -> String { if alert.state == .active && !alert.armed { return "Waiting to re-arm" }; if store.latestQuotes[alert.asset.key]?.isFresh != true && alert.state == .active { return "Waiting for current market data" }; return alert.state.rawValue.capitalized }
}

struct GlobalAlertBanner: View {
    @StateObject private var store = AlertStore.shared
    var body: some View {
        if let event = store.bannerEvent {
            HStack(spacing: 10) {
                Image(systemName: "bell.fill").foregroundStyle(.orange)
                Text("\(event.asset.symbol) reached \(event.target.description) \(event.currency.rawValue)")
                Button { store.bannerEvent = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain)
            }
            .padding(.horizontal, 14).padding(.vertical, 9)
            .background(.regularMaterial, in: Capsule()).shadow(radius: 5).padding(.top, 8)
            .task(id: event.id) { try? await Task.sleep(for: .seconds(6)); if store.bannerEvent?.id == event.id { store.bannerEvent = nil } }
        }
    }
}
