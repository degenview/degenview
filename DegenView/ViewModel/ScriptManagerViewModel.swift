import AppKit
import Foundation
import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class ScriptManagerViewModel: ObservableObject {
    struct Group: Identifiable {
        /// A script favorited within an "Indicators" group (say) appears in both the
        /// Favorites group and its type group. Each row therefore needs an id unique
        /// across the whole sidebar (group + script), not just the script's own id —
        /// otherwise List/ForEach conflate the two rows and rename state, focus, etc.
        /// land on whichever row SwiftUI decides is "the" row for that duplicated id.
        struct Row: Identifiable {
            let id: String
            let script: LocalScript
        }
        let id: String
        let title: String
        let rows: [Row]
    }
    @Published var scripts: [LocalScript] = []
    /// False until the first fetch lands, so an empty `scripts` isn't mistaken for "no scripts yet".
    @Published private(set) var hasLoaded = false
    @Published var selection: UUID?
    @Published var query = ""
    @Published var errorMessage: String?
    @Published private var collapsedGroups: Set<String> = []
    /// Keyed by `Group.Row.id`, not the script's own id — a favorited script has a row
    /// in two groups sharing one script id, and only the row actually clicked should
    /// enter rename mode.
    @Published var renamingRowID: String?
    private var lastRowClick: (rowID: String, date: Date)?

    /// Finder-style "slow double click" rename: a second click on an already-selected
    /// row, spaced out enough that AppKit wouldn't treat it as a real double click.
    func handleRowClick(rowID: String, scriptID: UUID) {
        let now = Date()
        defer { lastRowClick = (rowID, now) }
        guard selection == scriptID, let last = lastRowClick, last.rowID == rowID else { return }
        let interval = now.timeIntervalSince(last.date)
        if interval > 0.4 && interval < 1.5 { renamingRowID = rowID }
    }

    /// Why `raw` cannot name a script (invalid file name, or another script already has it),
    /// or nil when it can. Empty input has no message; callers disable their action instead.
    func nameProblem(for raw: String, excluding id: UUID? = nil) -> String? {
        switch ScriptNameValidator.validate(raw) {
        case .failure(.empty):
            return raw.isEmpty ? nil : ScriptNameError.empty.localizedDescription
        case .failure(let error):
            return error.localizedDescription
        case .success(let clean):
            let taken = scripts.contains {
                $0.id != id && $0.name.caseInsensitiveCompare(clean) == .orderedSame
            }
            return taken ? ScriptStoreError.nameConflict.localizedDescription : nil
        }
    }

    /// Validates before touching the store. An invalid name reports why and leaves the row in
    /// edit mode so it can be corrected.
    func commitRename(_ script: LocalScript, newName: String) {
        guard case .success(let clean) = ScriptNameValidator.validate(newName),
            nameProblem(for: newName, excluding: script.id) == nil
        else {
            errorMessage = nameProblem(for: newName, excluding: script.id) ?? ScriptNameError.empty.localizedDescription
            return
        }
        renamingRowID = nil
        guard clean != script.name else { return }
        Task {
            do {
                try await ScriptStore.shared.rename(id: script.id, to: clean)
                NotificationCenter.default.post(name: .localScriptsDidChange, object: script.id)
                await refresh()
            } catch { errorMessage = error.localizedDescription }
        }
    }

    func isExpanded(_ groupID: String) -> Binding<Bool> {
        Binding(
            get: { !self.collapsedGroups.contains(groupID) },
            set: { expanded in
                if expanded { self.collapsedGroups.remove(groupID) } else { self.collapsedGroups.insert(groupID) }
            }
        )
    }

    /// Scripts grouped for the sidebar: Favorites first (only when non-empty), then by type.
    var groups: [Group] {
        let matches = scripts.filter {
            query.isEmpty || $0.name.localizedCaseInsensitiveContains(query)
                || $0.source.localizedCaseInsensitiveContains(query)
        }
        func sorted(_ scripts: [LocalScript]) -> [LocalScript] { scripts.sorted { $0.modifiedAt > $1.modifiedAt } }
        func rows(for scripts: [LocalScript], groupID: String) -> [Group.Row] {
            scripts.map { Group.Row(id: "\(groupID)#\($0.id)", script: $0) }
        }
        let favorites = sorted(matches.filter(\.isFavorite))
        let byType: [(ScriptType, String)] = [
            (.indicator, "Indicators"), (.strategy, "Strategies"), (.library, "Libraries"),
        ]
        var result: [Group] = []
        if !favorites.isEmpty {
            result.append(Group(id: "favorites", title: "Favorites", rows: rows(for: favorites, groupID: "favorites")))
        }
        for (type, title) in byType {
            let scripts = sorted(matches.filter { $0.type == type })
            if !scripts.isEmpty {
                result.append(Group(id: type.rawValue, title: title, rows: rows(for: scripts, groupID: type.rawValue)))
            }
        }
        return result
    }

    func load() {
        Task { await refresh(selecting: WindowCoordinator.shared.takePendingScriptManagerSelection()) }
    }
    /// Selects a script opened from outside the manager, e.g. an imported `.pine` file.
    func select(_ id: UUID) {
        _ = WindowCoordinator.shared.takePendingScriptManagerSelection()
        Task { await refresh(selecting: id) }
    }
    /// The template declares an indicator; the type is re-detected from the code on save.
    func create(named name: String) {
        if let problem = nameProblem(for: name) {
            errorMessage = problem
            return
        }
        Task {
            do {
                let script = try await ScriptStore.shared.create(
                    name: name, type: .indicator, disambiguating: false)
                await refresh(selecting: script.id)
            } catch { errorMessage = error.localizedDescription }
        }
    }
    func toggleFavorite(_ script: LocalScript) {
        Task {
            do {
                try await ScriptStore.shared.setFavorite(id: script.id, !script.isFavorite)
                await refresh()
            } catch { errorMessage = error.localizedDescription }
        }
    }
    func showInFinder(_ script: LocalScript) {
        Task {
            do {
                guard let url = try await ScriptStore.shared.fileURL(for: script.id) else {
                    throw ScriptStoreError.missingScript
                }
                NSWorkspace.shared.activateFileViewerSelecting([url])
            } catch { errorMessage = error.localizedDescription }
        }
    }
    /// Saves a copy of the script's file wherever the user picks. Writes the bytes rather
    /// than copying the file so the library's script-id attribute doesn't travel along.
    func export(_ script: LocalScript) {
        let panel = NSSavePanel()
        panel.title = "Export Script"
        panel.nameFieldStringValue = "\(script.name).\(ScriptStore.fileExtension)"
        panel.allowedContentTypes = [UTType("com.cryptocharts.pine-script") ?? .plainText]
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let destination = panel.url else { return }
        Task {
            do {
                guard let source = try await ScriptStore.shared.fileURL(for: script.id) else {
                    throw ScriptStoreError.missingScript
                }
                try Data(contentsOf: source).write(to: destination, options: .atomic)
            } catch { errorMessage = error.localizedDescription }
        }
    }
    func delete(_ script: LocalScript) {
        Task {
            do {
                try await ScriptStore.shared.delete(id: script.id)
                if selection == script.id { selection = nil }
                await refresh()
            } catch { errorMessage = error.localizedDescription }
        }
    }
    private func refresh(selecting id: UUID? = nil) async {
        do {
            scripts = try await ScriptStore.shared.allScripts()
            hasLoaded = true
            if let id { selection = id }
            // The file may have been deleted or renamed away outside the app.
            if let current = selection, !scripts.contains(where: { $0.id == current }) { selection = nil }
        } catch { errorMessage = error.localizedDescription }
    }
}
