import SwiftUI

struct ScriptEditorView: View {
    @StateObject private var model: ScriptEditorViewModel
    init(scriptID: UUID) { _model = StateObject(wrappedValue: ScriptEditorViewModel(scriptID: scriptID)) }
    var body: some View {
        VStack(spacing: 0) {
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
        .onReceive(NotificationCenter.default.publisher(for: .localScriptsDidChange)) { _ in model.refreshName() }
    }
}
