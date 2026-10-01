import SwiftUI

/// The panel under the preview chart: the script's inputs, its strategy report, and its problems.
/// Only the tab bar is always visible; the content opens and closes beneath it.
struct ScriptPreviewDrawer: View {
    enum Tab: String, CaseIterable, Identifiable {
        case inputs = "Inputs"
        case report = "Report"
        case problems = "Problems"

        var id: String { rawValue }

        var symbol: String {
            switch self {
            case .inputs: "slider.horizontal.3"
            case .report: "chart.line.uptrend.xyaxis"
            case .problems: "exclamationmark.triangle"
            }
        }
    }

    @ObservedObject var preview: ScriptPreviewViewModel
    @ObservedObject var chart: ChartViewModel
    let editorDiagnostics: [PineDiagnostic]
    let editorHasErrors: Bool
    let tab: Tab

    /// Problems for whichever source the user is looking at: the editor's while it doesn't compile,
    /// the running script's (warnings, or the runtime failure) otherwise.
    var diagnostics: [PineDiagnostic] {
        editorHasErrors || preview.unsupportedReason != nil ? editorDiagnostics : chart.pineDiagnostics
    }

    var body: some View {
        Group {
            switch tab {
            case .inputs: inputs
            case .report: report
            case .problems: problems
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(.background)
    }

    // MARK: - Tabs

    @ViewBuilder private var inputs: some View {
        if preview.inputSchema.inputs.isEmpty {
            placeholder(
                "No inputs",
                "Declare one with input.int(), input.float(), input.bool() and friends, and it appears here.")
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    PineInputsView(schema: preview.inputSchema, values: preview.inputValues) { value, id in
                        preview.setInput(value, id: id)
                    }
                    HStack {
                        Spacer()
                        Button("Reset to Defaults") { preview.resetInputs() }
                            .disabled(preview.inputValues.isEmpty)
                    }
                }
                .padding(12)
            }
        }
    }

    @ViewBuilder private var report: some View {
        if chart.pineOutput.strategy != nil || !chart.pineOutput.alerts.isEmpty {
            ScrollView {
                PineStrategyReportView(report: chart.pineOutput.strategy, alerts: chart.pineOutput.alerts)
                    .padding(12)
            }
        } else {
            placeholder(
                "No report",
                "Strategies report their trades and equity curve here. Indicators show alerts they raise.")
        }
    }

    @ViewBuilder private var problems: some View {
        if diagnostics.isEmpty {
            placeholder("No problems", "Compile errors, warnings and runtime failures are listed here.")
        } else {
            PineDiagnosticsListView(diagnostics: diagnostics, maxHeight: nil)
                .padding(12)
        }
    }

    private func placeholder(_ title: String, _ message: String) -> some View {
        VStack(spacing: 4) {
            Text(title).font(.subheadline.weight(.semibold))
            Text(message)
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Always-visible strip of tabs under the chart. Picking a tab opens the drawer; picking the open
/// one again, or the chevron, closes it.
struct ScriptPreviewDrawerBar: View {
    @Binding var tab: ScriptPreviewDrawer.Tab
    @Binding var isOpen: Bool
    let problemCount: Int
    let hasErrors: Bool
    let status: String

    var body: some View {
        HStack(spacing: 2) {
            ForEach(ScriptPreviewDrawer.Tab.allCases) { candidate in
                tabButton(candidate)
            }
            Spacer(minLength: 8)
            Text(status)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
            Button {
                withAnimation(.snappy) { isOpen.toggle() }
            } label: {
                Image(systemName: "chevron.down")
                    .rotationEffect(.degrees(isOpen ? 0 : 180))
                    .frame(width: 22, height: 22)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help(isOpen ? "Hide panel" : "Show panel")
            .accessibilityLabel(isOpen ? "Hide panel" : "Show panel")
        }
        .padding(.horizontal, 8)
        .frame(height: 30)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
    }

    private func tabButton(_ candidate: ScriptPreviewDrawer.Tab) -> some View {
        let selected = isOpen && tab == candidate
        return Button {
            withAnimation(.snappy) {
                if selected {
                    isOpen = false
                } else {
                    tab = candidate
                    isOpen = true
                }
            }
        } label: {
            HStack(spacing: 5) {
                Image(systemName: candidate.symbol)
                Text(candidate.rawValue)
                if candidate == .problems, problemCount > 0 {
                    Text("\(problemCount)")
                        .font(.caption2.weight(.semibold).monospacedDigit())
                        .foregroundStyle(.white)
                        .padding(.horizontal, 5)
                        .background(hasErrors ? Color.red : Color.orange, in: Capsule())
                }
            }
            .font(.caption.weight(selected ? .semibold : .regular))
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(selected ? Color.accentColor.opacity(0.15) : .clear, in: RoundedRectangle(cornerRadius: 6))
            .foregroundStyle(selected ? Color.accentColor : .secondary)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
