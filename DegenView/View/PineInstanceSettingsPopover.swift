import SwiftUI

/// Inputs for one applied Pine instance, with explicit Apply/Cancel — no rebuild per
/// keystroke. Shared by the chart legend's gear icon and the chart settings sheet's
/// instance row; both just hand it an instance id.
struct PineInstanceSettingsPopover: View {
    @ObservedObject var viewModel: ChartViewModel
    let instanceID: UUID
    /// Marks the chart's saved layout dirty after Apply — inputs are persisted configuration.
    var onStyleChanged: () -> Void = {}
    @Environment(\.dismiss) private var dismiss
    @State private var draftInputs: [String: PineInputValue]

    init(viewModel: ChartViewModel, instanceID: UUID, onStyleChanged: @escaping () -> Void = {}) {
        self.viewModel = viewModel
        self.instanceID = instanceID
        self.onStyleChanged = onStyleChanged
        let current = viewModel.scriptInstances.first { $0.id == instanceID }?.inputs ?? [:]
        _draftInputs = State(initialValue: current)
    }

    private var result: PineInstanceResult? { viewModel.pineResults[instanceID] }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(result?.declaration?.title ?? "Script").font(.headline)
            if let diagnostics = result?.diagnostics, !diagnostics.isEmpty {
                PineDiagnosticsListView(diagnostics: diagnostics, maxHeight: 90, showsHeader: false)
            }
            ScrollView {
                PineInputsView(schema: result?.inputSchema ?? PineInputSchema(), values: draftInputs) {
                    value, id in draftInputs[id] = value
                }
            }
            .frame(maxHeight: 320)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Apply") {
                    viewModel.setPineInstanceInputs(instanceID, inputs: draftInputs)
                    onStyleChanged()
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(16)
        .frame(width: 320)
    }
}
