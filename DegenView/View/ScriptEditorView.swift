import SwiftUI

struct ScriptEditorView: View {
    @StateObject private var model: ScriptEditorViewModel
    init(scriptID: UUID?) { _model = StateObject(wrappedValue: ScriptEditorViewModel(scriptID: scriptID)) }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Name")
                TextField("Script Name", text: $model.name)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 200)
                Spacer()
                Picker("Type", selection: $model.type) { ForEach(ScriptType.allCases) { Text($0.displayName).tag($0) } }
                    .frame(width: 130)
                if model.status == .error {
                    Label("Error", systemImage: "xmark.octagon.fill")
                        .foregroundStyle(.red)
                        .help(firstErrorMessage ?? "")
                }
            }
            .padding()
            .fixedSize(horizontal: false, vertical: true)
            .layoutPriority(2)
            .zIndex(1)
            if let message = firstErrorMessage {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal)
                    .padding(.bottom, 6)
            }
            Divider()
            LineNumberedTextEditorView(text: $model.source, diagnostics: model.diagnostics)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .frame(minHeight: 200)
                .clipped()
                .layoutPriority(1)
                .onChange(of: model.source) { model.changed() }
            Divider()
            HStack {
                Spacer()
                Button("Save") { model.save() }
                    .keyboardShortcut("s", modifiers: .command)
            }
            .padding()
            .fixedSize(horizontal: false, vertical: true)
        }
        .navigationTitle(model.name + (model.isDirty ? " — Edited" : ""))
        .task { model.load() }
    }
    private var firstErrorMessage: String? {
        guard let diagnostic = model.diagnostics.first(where: { $0.severity == .error }) else { return nil }
        return "Line \(diagnostic.range.start.line): \(diagnostic.message)"
    }
}
