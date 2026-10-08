import Foundation

/// The saved scripts a chart can apply, shared by the chart settings sheet and the header's add
/// menu so every chart card does not query the store on its own.
@MainActor
final class SavedScriptsModel: ObservableObject {
    static let shared = SavedScriptsModel()

    @Published private(set) var scripts: [LocalScript] = []
    @Published private(set) var loadError: String?
    private var observer: NSObjectProtocol?
    private var hasLoaded = false

    init() {
        observer = NotificationCenter.default.addObserver(
            forName: .localScriptsDidChange, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in await self?.reload() }
        }
    }

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }

    func loadIfNeeded() async {
        guard !hasLoaded else { return }
        await reload()
    }

    func reload() async {
        do {
            scripts = try await ScriptStore.shared.allScripts()
            loadError = nil
        } catch {
            loadError = error.localizedDescription
        }
        hasLoaded = true
    }

    /// Libraries only export code to other scripts, so they are never offered for a chart.
    var indicatorScripts: [LocalScript] { appliable(of: .indicator) }
    var strategyScripts: [LocalScript] { appliable(of: .strategy) }
    var hasAppliableScripts: Bool { !indicatorScripts.isEmpty || !strategyScripts.isEmpty }

    func script(withID id: UUID) -> LocalScript? { scripts.first { $0.id == id } }

    private func appliable(of type: ScriptType) -> [LocalScript] {
        scripts
            .filter { $0.type == type }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    /// Applies `script` to `viewModel`; returns why it could not be added, nil on success.
    func add(_ script: LocalScript, to viewModel: ChartViewModel) -> String? {
        guard
            viewModel.addPineInstance(
                scriptID: script.id, revisionID: script.latestRevisionID ?? UUID(), source: script.source) != nil
        else {
            return "\"\(script.name)\" didn't compile, so it wasn't added. Fix it in the Script Manager."
        }
        return nil
    }
}
