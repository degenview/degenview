import SwiftUI

struct ScriptManagerView: View {
    @StateObject private var model = ScriptManagerViewModel()
    @State private var pendingDelete: LocalScript?

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Text("Script Manager").font(.headline)
                Spacer()
                Button { model.create() } label: {
                    Label("New Script", systemImage: "plus")
                }
            }
            .padding(10)
            Divider()

            HSplitView {
                VStack(spacing: 0) {
                    List(selection: $model.selection) {
                        ForEach(model.groups) { group in
                            Section(isExpanded: model.isExpanded(group.id)) {
                                ForEach(group.rows) { row in
                                    ScriptRow(script: row.script, rowID: row.id, model: model, onDelete: { pendingDelete = $0 })
                                        .tag(row.script.id)
                                }
                            } header: {
                                Text(group.title)
                            }
                        }
                    }
                    .listStyle(.sidebar)
                    .frame(maxHeight: .infinity)

                    Divider()
                    HStack(spacing: 6) {
                        Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                        TextField("Search name or source", text: $model.query)
                            .textFieldStyle(.plain)
                    }
                    .padding(8)
                }
                .frame(minWidth: 200, idealWidth: 240, maxWidth: 320)

                if let id = model.selection {
                    ScriptEditorView(scriptID: id)
                        .id(id)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    Text("Select a script")
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        }
        // Keep AppKit's unified toolbar row alive for this native tab. The chart
        // tab's controls must disappear here, but removing toolbar content entirely
        // collapses the titlebar and makes the window jump vertically.
        .toolbar {
            ToolbarItem(placement: .automatic) {
                Color.clear
                    .frame(width: 1, height: 28)
                    .accessibilityHidden(true)
                    .allowsHitTesting(false)
            }
        }
        .task { model.load() }
        .onReceive(NotificationCenter.default.publisher(for: .localScriptsDidChange)) { _ in model.load() }
        .onReceive(NotificationCenter.default.publisher(for: .selectScriptInManager)) { note in
            if let id = note.object as? UUID { model.select(id) }
        }
        .background(WindowAccessor { WindowCoordinator.shared.registerAuxiliaryTab($0) })
        .alert("Script Manager", isPresented: .constant(model.errorMessage != nil)) {
            Button("OK") { model.errorMessage = nil }
        } message: {
            Text(model.errorMessage ?? "")
        }
        .confirmationDialog(
            "Delete Script",
            isPresented: Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }),
            presenting: pendingDelete
        ) { script in
            Button("Delete \"\(script.name)\"", role: .destructive) {
                model.delete(script)
                pendingDelete = nil
            }
            Button("Cancel", role: .cancel) { pendingDelete = nil }
        } message: { script in
            Text("The script file will be moved to the Trash.")
        }
    }
}

private struct ScriptRow: View {
    let script: LocalScript
    let rowID: String
    @ObservedObject var model: ScriptManagerViewModel
    let onDelete: (LocalScript) -> Void

    @State private var draftName = ""
    @FocusState private var isRenameFocused: Bool

    var body: some View {
        HStack {
            if model.renamingRowID == rowID {
                TextField("Name", text: $draftName)
                    .textFieldStyle(.plain)
                    .focused($isRenameFocused)
                    .onSubmit { model.commitRename(script, newName: draftName) }
                    .onExitCommand { model.renamingRowID = nil }
                    .onAppear {
                        draftName = script.name
                        isRenameFocused = true
                    }
            } else {
                Text(script.name)
            }
            Spacer()
            Button {
                model.toggleFavorite(script)
            } label: {
                Image(systemName: script.isFavorite ? "star.fill" : "star")
                    .foregroundStyle(script.isFavorite ? .yellow : .secondary)
            }
            .buttonStyle(.plain)
        }
        .contentShape(Rectangle())
        .simultaneousGesture(TapGesture().onEnded { model.handleRowClick(rowID: rowID, scriptID: script.id) })
        .contextMenu {
            Button(script.isFavorite ? "Remove Favorite" : "Favorite") { model.toggleFavorite(script) }
            Divider()
            Button("Show in Finder") { model.showInFinder(script) }
            Button("Export…") { model.export(script) }
            Divider()
            Button("Delete", role: .destructive) { onDelete(script) }
        }
    }
}
