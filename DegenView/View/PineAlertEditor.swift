import SwiftUI

/// Creates a notification subscription for the alerts a chart's applied script raises.
struct PineAlertEditor: View {
    @ObservedObject var viewModel: ChartViewModel
    @StateObject private var store = PineAlertStore.shared
    @Environment(\.dismiss) private var dismiss
    @State private var note = ""
    @State private var scriptName = ""
    @State private var callSites: [PineAlertCallSite] = []

    private var dataset: PineDatasetKey { viewModel.pineAlertDataset }

    private var duplicateExists: Bool {
        store.subscriptions(forChart: viewModel.chartID).contains {
            $0.isActive && $0.sourceHash == viewModel.appliedSourceHash && $0.watches(dataset)
        }
    }

    private var hint: String? {
        if viewModel.appliedSourceHash == nil { return "Apply a script to this chart first." }
        if callSites.isEmpty { return "The applied script has no alert() or alertcondition() calls." }
        if duplicateExists { return "This script already has an active alert on this chart." }
        return nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Create Script Alert").font(.title2.bold())

            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
                GridRow {
                    Text("Script").foregroundStyle(.secondary)
                    Text(scriptName.isEmpty ? "—" : scriptName)
                }
                GridRow {
                    Text("Symbol").foregroundStyle(.secondary)
                    Text(dataset.symbolKey)
                }
                GridRow {
                    Text("Timeframe").foregroundStyle(.secondary)
                    Text(dataset.timeframe)
                }
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Alert calls").font(.caption.weight(.semibold))
                if callSites.isEmpty {
                    Text("None").font(.caption).foregroundStyle(.secondary)
                } else {
                    ForEach(Array(callSites.enumerated()), id: \.offset) { _, site in
                        Text(describe(site)).font(.caption.monospaced())
                    }
                }
            }

            TextField("Note (optional)", text: $note).textFieldStyle(.roundedBorder)

            Text(
                "Fires only while this chart shows this symbol and timeframe and its live feed is running. "
                    + "Editing the script pauses the alert until you re-arm it."
            )
            .font(.caption).foregroundStyle(.secondary)

            if let hint {
                Label(hint, systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
            }

            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Create Alert") {
                    PineAlertCoordinator.shared.subscribe(
                        viewModel, scriptID: viewModel.scriptInstances.first?.scriptID,
                        scriptName: scriptName, note: note)
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .disabled(hint != nil)
            }
        }
        .padding(22).frame(width: 430)
        .task { await load() }
    }

    private func describe(_ site: PineAlertCallSite) -> String {
        let name = site.isCondition ? "alertcondition" : "alert"
        guard let frequency = site.frequency else { return "\(name)() · frequency set by the script" }
        return "\(name)() · \(frequency.rawValue)"
    }

    private func load() async {
        guard let source = viewModel.pineConfiguration?.appliedSource, !source.isEmpty else { return }
        let program = PineCompiler.compile(source: source, libraries: PineLibraryRegistry.shared)
        callSites = program.alertCallSites
        if let id = viewModel.scriptInstances.first?.scriptID,
            let saved = try? await ScriptStore.shared.script(id: id)
        {
            scriptName = saved.name
        } else {
            scriptName = program.declaration.title
        }
    }
}
