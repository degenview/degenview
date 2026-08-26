import Foundation

@MainActor
final class ScriptEditorViewModel: ObservableObject {
    @Published private(set) var scriptID: UUID?
    @Published var name = ""
    @Published var type: ScriptType = .indicator
    @Published var source = ""
    @Published var status: CompileStatus = .notCompiled
    @Published var diagnostics: [PineDiagnostic] = []
    @Published var isDirty = false
    @Published var didSave = false
    @Published var errorMessage: String?
    private var savedSource = ""
    private var draftTask: Task<Void, Never>?

    init(scriptID: UUID?) {
        self.scriptID = scriptID
        if scriptID == nil {
            name = "Untitled"
            source = ScriptStore.template(for: .indicator, title: "Untitled")
            isDirty = true
        }
    }
    func load() {
        guard let scriptID else { return }
        Task {
            do {
                guard let script = try await ScriptStore.shared.script(id: scriptID) else { return }
                name = script.name
                type = script.type
                source = script.source
                savedSource = script.source
                status =
                    script.compileRecord?.compilerVersion == ScriptStore.compilerVersion
                    ? script.compileRecord?.status ?? .notCompiled : .notCompiled
                diagnostics = script.compileRecord?.diagnostics ?? []
                if let draft = try await ScriptStore.shared.draft(id: scriptID), draft.source != script.source {
                    source = draft.source
                    isDirty = true
                }
            } catch { errorMessage = error.localizedDescription }
        }
    }
    func changed() {
        isDirty = source != savedSource
        compile()
        guard let scriptID else { return }
        draftTask?.cancel()
        let draft = ScriptDraft(scriptID: scriptID, source: source, modifiedAt: Date(), basedOnRevisionID: nil)
        draftTask = Task {
            try? await Task.sleep(for: .seconds(1))
            guard !Task.isCancelled else { return }
            try? await ScriptStore.shared.saveDraft(draft)
        }
    }
    func compile() {
        let result = PineCompiler.compile(source: source)
        diagnostics = result.diagnostics
        status =
            result.diagnostics.contains { $0.severity == .error }
            ? .error : (result.diagnostics.contains { $0.severity == .warning } ? .warning : .valid)
    }
    func save() {
        compile()
        guard !diagnostics.contains(where: { $0.severity == .error }) else { return }
        let requestedType = type
        Task {
            do {
                let id: UUID
                if let scriptID {
                    id = scriptID
                } else {
                    let created = try await ScriptStore.shared.create(name: name, type: requestedType, source: source)
                    id = created.id
                    scriptID = id
                    name = created.name
                }
                let result = try await ScriptStore.shared.save(id: id, name: name, type: requestedType, source: source)
                savedSource = result.source
                isDirty = false
                status = result.compileRecord?.status ?? .notCompiled
                diagnostics = result.compileRecord?.diagnostics ?? []
                NotificationCenter.default.post(name: .localScriptsDidChange, object: result.id)
                didSave = true
            } catch { errorMessage = error.localizedDescription }
        }
    }
}
