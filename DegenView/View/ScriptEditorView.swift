import SwiftUI

struct ScriptEditorView: View {
    @StateObject private var model: ScriptEditorViewModel
    @Environment(\.dismissWindow) private var dismissWindow
    init(scriptID: UUID?) { _model = StateObject(wrappedValue: ScriptEditorViewModel(scriptID: scriptID)) }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Name")
                TextField("Script Name", text: $model.name)
                    .textFieldStyle(.roundedBorder)
                    .frame(minWidth: 220)
                Picker("Type", selection: $model.type) { ForEach(ScriptType.allCases) { Text($0.displayName).tag($0) } }.frame(width: 130)
                Text(model.status.rawValue.capitalized).foregroundStyle(.secondary)
            }
            .padding()
            .fixedSize(horizontal: false, vertical: true)
            .layoutPriority(2)
            .zIndex(1)
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
                Button("Validate") { model.compile() }
                Button("Save") { model.save() }
                    .keyboardShortcut("s", modifiers: .command)
            }
            .padding()
            .fixedSize(horizontal: false, vertical: true)
        }
        .navigationTitle(model.name + (model.isDirty ? " — Edited" : ""))
        .task { model.load() }
        .onChange(of: model.didSave) { _, didSave in
            if didSave { dismissWindow() }
        }
    }
}
