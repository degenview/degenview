import Foundation

@MainActor
final class ScriptEditorViewModel: ObservableObject {
    let scriptID: UUID
    @Published private(set) var name = ""
    @Published var source = ""
    @Published var status: CompileStatus = .notCompiled
    @Published var diagnostics: [PineDiagnostic] = []
    @Published var isDirty = false
    @Published var errorMessage: String?
    /// A request for the editor to scroll to and flash a source position; each click makes a new one.
    @Published private(set) var reveal: Reveal?

    struct Reveal: Equatable {
        let id = UUID()
        let line: Int
        let column: Int
    }
    private var savedSource = ""
    private var draftTask: Task<Void, Never>?

    /// Asks the editor to show where `diagnostic` points.
    func reveal(_ diagnostic: PineDiagnostic) {
        reveal = Reveal(line: diagnostic.range.start.line, column: diagnostic.range.start.column)
    }

    init(scriptID: UUID) {
        self.scriptID = scriptID
    }
    func load() {
        Task {
            do {
                guard let script = try await ScriptStore.shared.script(id: scriptID) else { return }
                name = script.name
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
    /// Picks up a rename made from the sidebar so the window title stays current.
    func refreshName() {
        Task {
            if let script = try? await ScriptStore.shared.script(id: scriptID) { name = script.name }
        }
    }
    func changed() {
        isDirty = source != savedSource
        compile()
        draftTask?.cancel()
        let draft = ScriptDraft(scriptID: scriptID, source: source, modifiedAt: Date(), basedOnRevisionID: nil)
        draftTask = Task {
            try? await Task.sleep(for: .seconds(1))
            guard !Task.isCancelled else { return }
            try? await ScriptStore.shared.saveDraft(draft)
        }
    }
    func compile() {
        let result = PineCompiler.compile(source: source, libraries: PineLibraryRegistry.shared)
        diagnostics = result.diagnostics
        status =
            result.diagnostics.contains { $0.severity == .error }
            ? .error : (result.diagnostics.contains { $0.severity == .warning } ? .warning : .valid)
    }
    func save() {
        compile()
        Task {
            do {
                let result = try await ScriptStore.shared.save(id: scriptID, source: source)
                savedSource = result.source
                isDirty = false
                status = result.compileRecord?.status ?? .notCompiled
                diagnostics = result.compileRecord?.diagnostics ?? []
                NotificationCenter.default.post(name: .localScriptsDidChange, object: result.id)
            } catch { errorMessage = error.localizedDescription }
        }
    }
}
