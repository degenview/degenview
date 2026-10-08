import SwiftUI

/// The Scripts page of the chart settings sheet: add Pine scripts to the chart, see how each is doing,
/// and show, tune, alert on, reorder or remove them. Like the rest of the sheet nothing is saved
/// separately — every action writes through to the chart.
struct ChartScriptsTab: View {
    @ObservedObject var viewModel: ChartViewModel
    let onStyleChanged: () -> Void

    @ObservedObject private var pineAlerts = PineAlertStore.shared
    @ObservedObject private var saved = SavedScriptsModel.shared
    @State private var addScriptError: String?
    @State private var editingInstanceID: UUID?
    /// The applied script awaiting a "remove" confirmation — only asked when removing it also deletes an alert.
    @State private var instancePendingRemoval: UUID?
    @State private var alertTarget: AlertTarget?

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header
                if let scriptLoadError = saved.loadError {
                    NoticeCard(
                        systemImage: "xmark.octagon.fill", tint: .red, title: "Couldn't load your scripts",
                        detail: scriptLoadError)
                }
                if let addScriptError {
                    NoticeCard(
                        systemImage: "exclamationmark.triangle.fill", tint: .orange,
                        title: "Couldn't add script", detail: addScriptError)
                }
                content
                backtests
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .confirmationDialog(
            "Remove \(instancePendingRemoval.map(displayName(of:)) ?? "script")?",
            isPresented: Binding(
                get: { instancePendingRemoval != nil },
                set: { if !$0 { instancePendingRemoval = nil } }),
            titleVisibility: .visible
        ) {
            Button("Remove Script and Alert", role: .destructive) {
                if let id = instancePendingRemoval { removeInstance(id) }
                instancePendingRemoval = nil
            }
        } message: {
            Text("This script has an alert on this chart. Removing the script deletes the alert too.")
        }
        .sheet(item: $alertTarget) { PineAlertEditor(viewModel: viewModel, instanceID: $0.id) }
        .task { await saved.loadIfNeeded() }
    }

    // MARK: - Header

    private var header: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Scripts").font(.title3.weight(.semibold))
                Text("Pine scripts running on this chart.").font(.subheadline).foregroundStyle(.secondary)
            }
            Spacer(minLength: 12)
            if hasSavedScripts { addMenu }
        }
    }

    private var addMenu: some View {
        Menu {
            if !indicatorScripts.isEmpty {
                Section("Indicators") {
                    ForEach(indicatorScripts) { script in
                        Button(script.name) { addScriptToChart(script) }
                    }
                }
            }
            if !strategyScripts.isEmpty {
                Section("Strategies") {
                    ForEach(strategyScripts) { script in
                        Button(script.name) { addScriptToChart(script) }
                    }
                }
            }
            Divider()
            Button("Script Manager…", action: openScriptManager)
        } label: {
            Label("Add Script", systemImage: "plus")
        }
        .menuStyle(.button)
        .controlSize(.large)
        .fixedSize()
        .help("Apply a script to this chart — add the same one again with different inputs")
    }

    // MARK: - Applied list

    @ViewBuilder private var content: some View {
        if viewModel.scriptInstances.isEmpty {
            if hasSavedScripts {
                ScriptsEmptyState(
                    systemImage: "chart.xyaxis.line", title: "No scripts on this chart",
                    message: "Choose Add Script to overlay an indicator or strategy on the candles.")
            } else if saved.loadError == nil {
                ScriptsEmptyState(
                    systemImage: "curlybraces", title: "No scripts yet",
                    message: "Write one in the Script Manager and it shows up here."
                ) {
                    Button("Open Script Manager", action: openScriptManager)
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                }
            }
        } else {
            VStack(alignment: .leading, spacing: 8) {
                listHeader
                VStack(spacing: 0) {
                    ForEach(Array(viewModel.scriptInstances.enumerated()), id: \.element.id) { index, instance in
                        if index > 0 { Divider().padding(.leading, 54) }
                        row(for: instance, at: index)
                    }
                }
                .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .stroke(.separator.opacity(0.45), lineWidth: 1)
                }
            }
        }
    }

    /// Every script on a chart runs over the same candles, so the bar count and live state are shown
    /// once here rather than on each row.
    private var listHeader: some View {
        HStack(spacing: 8) {
            Text("Applied").font(.subheadline.weight(.semibold))
            Text("\(viewModel.scriptInstances.count)")
                .font(.caption.weight(.semibold).monospacedDigit())
                .foregroundStyle(.secondary)
                .padding(.horizontal, 7)
                .padding(.vertical, 1)
                .background(.quaternary, in: Capsule())
            Spacer()
            if let summary = barSummary {
                SettingsStatusBadge(text: summary.text, tone: summary.isLive ? .good : .neutral)
            }
        }
    }

    private var barSummary: (text: String, isLive: Bool)? {
        let results = viewModel.pineResults.values.filter { $0.output.barCount > 0 }
        guard let bars = results.map(\.output.barCount).max() else { return nil }
        let isLive = results.contains { $0.isLive }
        let count = "\(bars.formatted()) bars"
        return (isLive ? "Live · \(count)" : count, isLive)
    }

    private func row(for instance: ChartScriptInstance, at index: Int) -> some View {
        let result = viewModel.pineResults[instance.id]
        let id = instance.id
        return ScriptInstanceRow(
            title: displayName(of: id),
            kind: result?.declaration?.type ?? savedScript(for: instance)?.type ?? .indicator,
            isOverlay: result?.declaration?.overlay,
            state: result?.state ?? .evaluating,
            problem: result?.diagnostics.first { $0.severity == .error }?.message,
            isVisible: instance.isVisible,
            alert: alertControl(for: instance),
            isEditingInputs: Binding(
                get: { editingInstanceID == id },
                set: { isEditing in
                    if isEditing {
                        editingInstanceID = id
                    } else if editingInstanceID == id {
                        editingInstanceID = nil
                    }
                }),
            onToggleVisible: { viewModel.setPineInstanceVisible(id, isVisible: !instance.isVisible) },
            onAlert: { handleAlertTap(instance) },
            onRemove: { requestRemoval(of: instance) },
            onMoveUp: index > 0 ? { move(id, to: index - 1) } : nil,
            onMoveDown: index < viewModel.scriptInstances.count - 1 ? { move(id, to: index + 1) } : nil
        ) {
            PineInstanceSettingsPopover(viewModel: viewModel, instanceID: id, onStyleChanged: onStyleChanged)
        }
    }

    // MARK: - Backtests

    /// One report per applied strategy, so several strategies never hide behind "the first one".
    @ViewBuilder private var backtests: some View {
        let strategies = viewModel.scriptInstances.compactMap { instance in
            viewModel.pineResults[instance.id].flatMap { result in
                result.output.strategy.map { (instance: instance, report: $0, alerts: result.output.alerts) }
            }
        }
        if !strategies.isEmpty {
            VStack(alignment: .leading, spacing: 12) {
                Text("Backtest").font(.title3.weight(.semibold))
                ForEach(strategies, id: \.instance.id) { entry in
                    VStack(alignment: .leading, spacing: 6) {
                        if strategies.count > 1 {
                            Text(displayName(of: entry.instance.id))
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(.secondary)
                        }
                        PineStrategyReportView(report: entry.report, alerts: entry.alerts)
                    }
                }
            }
        }
    }

    // MARK: - Alerts

    private func alertControl(for instance: ChartScriptInstance) -> ScriptAlertControl {
        if let existing = existingAlert(for: instance) {
            let status = pineAlerts.status(of: existing)
            return .existing(statusText: status.text, tone: status.tone)
        }
        return .create(blockedReason: alertBlocker(for: instance))
    }

    private func existingAlert(for instance: ChartScriptInstance) -> PineAlertSubscription? {
        pineAlerts.subscription(
            forChart: viewModel.chartID, instanceID: instance.id, dataset: viewModel.pineAlertDataset)
    }

    private func handleAlertTap(_ instance: ChartScriptInstance) {
        if let existing = existingAlert(for: instance) {
            openAlert(existing)
        } else {
            alertTarget = AlertTarget(id: instance.id)
        }
    }

    /// Why an alert cannot be created for `instance` right now, nil when it can.
    private func alertBlocker(for instance: ChartScriptInstance) -> String? {
        guard let result = viewModel.pineResults[instance.id], viewModel.pineInstanceSourceHashes[instance.id] != nil
        else { return "Still loading this script…" }
        if result.diagnostics.contains(where: { $0.severity == .error }) {
            return "Fix this script's errors before creating an alert."
        }
        if result.alertCallCount == 0 { return "This script has no alert() or alertcondition() calls." }
        return nil
    }

    // MARK: - Scripts

    private var indicatorScripts: [LocalScript] { saved.indicatorScripts }
    private var strategyScripts: [LocalScript] { saved.strategyScripts }
    private var hasSavedScripts: Bool { saved.hasAppliableScripts }

    private func savedScript(for instance: ChartScriptInstance) -> LocalScript? {
        saved.script(withID: instance.scriptID)
    }

    /// The script's declared title, else its saved name, so a row never reads just "Script".
    private func displayName(of instanceID: UUID) -> String {
        if let title = viewModel.pineResults[instanceID]?.declaration?.title { return title }
        guard let instance = viewModel.scriptInstances.first(where: { $0.id == instanceID }) else { return "Script" }
        return savedScript(for: instance)?.name ?? "Script"
    }

    private func addScriptToChart(_ script: LocalScript) {
        addScriptError = saved.add(script, to: viewModel)
        if addScriptError == nil { onStyleChanged() }
    }

    private func move(_ id: UUID, to index: Int) {
        viewModel.movePineInstance(id, toIndex: index)
        onStyleChanged()
    }

    private func requestRemoval(of instance: ChartScriptInstance) {
        if existingAlert(for: instance) != nil {
            instancePendingRemoval = instance.id
        } else {
            removeInstance(instance.id)
        }
    }

    private func removeInstance(_ id: UUID) {
        viewModel.removePineInstance(id)
        onStyleChanged()
    }

    // MARK: - Navigation

    /// Closes this sheet, which would otherwise block the new tab, then opens the Script Manager.
    private func openScriptManager() {
        let scriptID = viewModel.scriptInstances.first?.scriptID
        // While the sheet is key the coordinator falls back to the chart window beneath it.
        WindowCoordinator.shared.prepareAuxiliaryTab()
        dismiss()
        DispatchQueue.main.async { WindowCoordinator.shared.openScriptManager(selecting: scriptID) }
    }

    /// Closes this sheet, then opens the Alerts window on the alert. Settings apply as they are
    /// changed, so nothing is lost by closing.
    private func openAlert(_ subscription: PineAlertSubscription) {
        dismiss()
        DispatchQueue.main.async { WindowCoordinator.shared.openAlerts(showing: subscription.id) }
    }
}

/// Which applied indicator the alert editor sheet is open for.
private struct AlertTarget: Identifiable {
    let id: UUID
}

/// A centred card for "nothing here yet", with an optional call to action below the message.
private struct ScriptsEmptyState<Actions: View>: View {
    let systemImage: String
    let title: String
    let message: String
    @ViewBuilder var actions: () -> Actions

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: systemImage)
                .font(.system(size: 20, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: 48, height: 48)
                .background(.quaternary.opacity(0.6), in: Circle())
                .accessibilityHidden(true)
            VStack(spacing: 3) {
                Text(title).font(.headline)
                Text(message)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
            actions()
        }
        .padding(.vertical, 36)
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(.separator.opacity(0.5), style: StrokeStyle(lineWidth: 1, dash: [5, 4]))
        }
        .accessibilityElement(children: .contain)
    }
}

extension ScriptsEmptyState where Actions == EmptyView {
    fileprivate init(systemImage: String, title: String, message: String) {
        self.init(systemImage: systemImage, title: title, message: message) { EmptyView() }
    }
}
