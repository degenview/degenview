import Foundation

@MainActor
final class ScriptManagerViewModel: ObservableObject {
    enum Section: String, CaseIterable, Identifiable {
        case all = "All Scripts", favorites = "Favorites", recent = "Recent"
        case indicators = "Indicators", strategies = "Strategies", libraries = "Libraries"
        var id: String { rawValue }
    }
    @Published var scripts: [LocalScript] = []
    @Published var selection: UUID?
    @Published var section: Section = .all
    @Published var query = ""
    @Published var errorMessage: String?

    var filtered: [LocalScript] {
        scripts.filter { script in
            let sectionMatch: Bool
            switch section {
            case .all: sectionMatch = true
            case .favorites: sectionMatch = script.isFavorite
            case .recent: sectionMatch = script.lastOpenedAt != nil
            case .indicators: sectionMatch = script.type == .indicator
            case .strategies: sectionMatch = script.type == .strategy
            case .libraries: sectionMatch = script.type == .library
            }
            return sectionMatch && (query.isEmpty || script.name.localizedCaseInsensitiveContains(query)
                || script.source.localizedCaseInsensitiveContains(query))
        }.sorted { $0.modifiedAt > $1.modifiedAt }
    }

    func load() { Task { do { scripts = try await ScriptStore.shared.allScripts() } catch { errorMessage = error.localizedDescription } } }
    func create(type: ScriptType = .indicator) {
        Task { do {
            let script = try await ScriptStore.shared.create(name: "Untitled", type: type)
            await refresh(selecting: script.id)
        } catch { errorMessage = error.localizedDescription } }
    }
    func toggleFavorite(_ script: LocalScript) {
        Task { do { try await ScriptStore.shared.setFavorite(id: script.id, !script.isFavorite); await refresh() }
        catch { errorMessage = error.localizedDescription } }
    }
    func delete(_ script: LocalScript) {
        Task { do { try await ScriptStore.shared.delete(id: script.id); await refresh() }
        catch { errorMessage = error.localizedDescription } }
    }
    private func refresh(selecting id: UUID? = nil) async {
        do { scripts = try await ScriptStore.shared.allScripts(); if let id { selection = id } }
        catch { errorMessage = error.localizedDescription }
    }
}
