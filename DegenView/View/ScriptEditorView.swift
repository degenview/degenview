import SwiftUI

struct ScriptEditorView: View {
    @ObservedObject var model: ScriptEditorViewModel

    var body: some View {
        VStack(spacing: 0) {
            LineNumberedTextEditorView(
                text: $model.source, diagnostics: model.diagnostics, reveal: model.reveal)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .frame(minHeight: 120)
                .clipped()
                .layoutPriority(1)
                .onChange(of: model.source) { model.changed() }
            Divider()
            HStack(spacing: 10) {
                ScriptStatusChip(status: model.status, diagnostics: model.diagnostics)
                Spacer()
                Button("Save") { model.save() }
                    .keyboardShortcut("s", modifiers: .command)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .fixedSize(horizontal: false, vertical: true)
        }
        .navigationTitle(model.name + (model.isDirty ? " — Edited" : ""))
        .task { model.load() }
        .onReceive(NotificationCenter.default.publisher(for: .localScriptsDidChange)) { _ in model.refreshName() }
    }
}

/// The editor's compile state at a glance: counts what the editor underlines.
private struct ScriptStatusChip: View {
    let status: CompileStatus
    let diagnostics: [PineDiagnostic]

    var body: some View {
        Label(text, systemImage: symbol)
            .font(.caption)
            .foregroundStyle(tint)
            .labelStyle(.titleAndIcon)
            .help("Compile status of the code in the editor")
    }

    private var errors: Int { diagnostics.filter { $0.severity == .error }.count }
    private var warnings: Int { diagnostics.filter { $0.severity == .warning }.count }

    private var text: String {
        switch status {
        case .notCompiled: "Not compiled"
        case .valid: "No problems"
        case .warning: warnings == 1 ? "1 warning" : "\(warnings) warnings"
        case .error: errors == 1 ? "1 error" : "\(errors) errors"
        }
    }

    private var symbol: String {
        switch status {
        case .notCompiled: "circle.dashed"
        case .valid: "checkmark.circle.fill"
        case .warning: "exclamationmark.triangle.fill"
        case .error: "xmark.octagon.fill"
        }
    }

    private var tint: Color {
        switch status {
        case .notCompiled: .secondary
        case .valid: .green
        case .warning: PineDiagnosticSeverity.warning.color
        case .error: PineDiagnosticSeverity.error.color
        }
    }
}
