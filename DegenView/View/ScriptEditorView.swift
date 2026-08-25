import SwiftUI

struct ScriptEditorView: View {
    @StateObject private var model: ScriptEditorViewModel
    @Environment(\.dismissWindow) private var dismissWindow
    init(scriptID: UUID?) { _model = StateObject(wrappedValue: ScriptEditorViewModel(scriptID: scriptID)) }
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                TextField("Script Name", text: $model.name).textFieldStyle(.plain).font(.title2)
                Picker("Type", selection: $model.type) { ForEach(ScriptType.allCases) { Text($0.displayName).tag($0) } }.frame(width: 130)
                Text(model.status.rawValue.capitalized).foregroundStyle(.secondary)
            }.padding()
            Divider()
            TextEditor(text: $model.source).font(.system(.body, design: .monospaced)).onChange(of: model.source) { model.changed() }
            if !model.diagnostics.isEmpty {
                Divider(); List(model.diagnostics) { diagnostic in
                    Label("Line \(diagnostic.range.start.line): \(diagnostic.message)", systemImage: diagnostic.severity == .error ? "xmark.circle" : "exclamationmark.triangle")
                }.frame(minHeight: 90, maxHeight: 180)
            }
        }
        .navigationTitle(model.name + (model.isDirty ? " — Edited" : ""))
        .toolbar { Button("Compile") { model.compile() }; Button("Save") { model.save() }.keyboardShortcut("s", modifiers: .command) }
        .task { model.load() }
        .onChange(of: model.didSave) { _, didSave in
            if didSave { dismissWindow() }
        }
    }
}
