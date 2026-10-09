import SwiftUI

/// Inputs and Style for one applied Pine instance, with explicit Apply/Cancel — no rebuild per
/// keystroke. Shared by the chart legend's gear icon and the chart settings sheet's
/// instance row; both just hand it an instance id.
struct PineInstanceSettingsPopover: View {
    @ObservedObject var viewModel: ChartViewModel
    let instanceID: UUID
    /// Marks the chart's saved layout dirty after Apply — inputs are persisted configuration.
    var onStyleChanged: () -> Void = {}
    @Environment(\.dismiss) private var dismiss
    @State private var draftInputs: [String: PineInputValue]
    @State private var draftStyle: [String: String]
    @State private var page = Page.inputs

    private enum Page: String, CaseIterable, Identifiable {
        case inputs = "Inputs"
        case style = "Style"
        var id: String { rawValue }
    }

    init(viewModel: ChartViewModel, instanceID: UUID, onStyleChanged: @escaping () -> Void = {}) {
        self.viewModel = viewModel
        self.instanceID = instanceID
        self.onStyleChanged = onStyleChanged
        let current = viewModel.scriptInstances.first { $0.id == instanceID }?.inputs ?? [:]
        _draftInputs = State(initialValue: current)
        _draftStyle = State(
            initialValue: viewModel.scriptInstances.first { $0.id == instanceID }?.styleOverrides ?? [:])
    }

    private var result: PineInstanceResult? { viewModel.pineResults[instanceID] }

    /// From the script's own output, so defaults show as the script has them.
    private var styleRows: [PineStyleRow] { result.map { PineStyleRows.rows(for: $0.output) } ?? [] }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(result?.declaration?.title ?? "Script").font(.headline)
            if !styleRows.isEmpty {
                Picker("Settings page", selection: $page) {
                    ForEach(Page.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
            }
            ScrollView {
                if page == .style && !styleRows.isEmpty {
                    PineStyleView(rows: styleRows, overrides: draftStyle) { draftStyle = $0 }
                } else {
                    PineInputsView(schema: result?.inputSchema ?? PineInputSchema(), values: draftInputs) {
                        value, id in draftInputs[id] = value
                    }
                }
            }
            .frame(maxHeight: 320)
            HStack {
                if page == .style {
                    Button("Reset Style") { draftStyle = [:] }
                        .disabled(draftStyle.isEmpty)
                }
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Apply") {
                    viewModel.setPineInstanceInputs(instanceID, inputs: draftInputs)
                    viewModel.setPineInstanceStyle(instanceID, overrides: draftStyle)
                    onStyleChanged()
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .padding(16)
        .frame(width: 440)
    }
}
