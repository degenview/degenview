import SwiftUI

struct ScriptManagerView: View {
    @StateObject private var model = ScriptManagerViewModel()
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Text("Script Manager").font(.headline)
                Spacer()
                TextField("Search name or source", text: $model.query)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 260)
                Button {
                    openWindow(value: ScriptEditorWindowID(scriptID: nil))
                } label: { Label("New Script", systemImage: "plus") }
            }
            .padding(10)
            Divider()

            HSplitView {
                List(ScriptManagerViewModel.Section.allCases, selection: $model.section) { section in
                    Label(section.rawValue, systemImage: icon(section)).tag(section)
                }
                .listStyle(.inset)
                .frame(minWidth: 180, idealWidth: 210, maxWidth: 280)

                Table(model.filtered, selection: $model.selection) {
                    TableColumn("Name") { script in Text(script.name) }
                    TableColumn("Type") { script in Text(script.type.displayName) }
                    TableColumn("Status") { script in Label(statusText(script), systemImage: statusIcon(script)) }
                    TableColumn("Modified") { script in Text(script.modifiedAt, style: .relative) }
                    TableColumn("Favorite") { script in Image(systemName: script.isFavorite ? "star.fill" : "star") }
                }
                .contextMenu(forSelectionType: UUID.self) { ids in
                    if let id = ids.first, let script = model.scripts.first(where: { $0.id == id }) {
                        Button("Open") { openWindow(value: ScriptEditorWindowID(scriptID: id)) }
                        Button(script.isFavorite ? "Remove Favorite" : "Favorite") { model.toggleFavorite(script) }
                        Divider(); Button("Delete", role: .destructive) { model.delete(script) }
                    }
                } primaryAction: { ids in
                    if let id = ids.first { openWindow(value: ScriptEditorWindowID(scriptID: id)) }
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
        .background(WindowAccessor { WindowCoordinator.shared.registerAuxiliaryTab($0) })
        .alert("Script Manager", isPresented: .constant(model.errorMessage != nil)) {
            Button("OK") { model.errorMessage = nil }
        } message: { Text(model.errorMessage ?? "") }
    }
    private func icon(_ section: ScriptManagerViewModel.Section) -> String {
        switch section { case .all: "doc.text"; case .favorites: "star"; case .recent: "clock"; case .indicators: "waveform.path.ecg"; case .strategies: "chart.xyaxis.line"; case .libraries: "books.vertical" }
    }
    private func statusText(_ script: LocalScript) -> String { script.compileRecord?.status.rawValue.capitalized ?? "Not Compiled" }
    private func statusIcon(_ script: LocalScript) -> String {
        switch script.compileRecord?.status ?? .notCompiled { case .notCompiled: "circle.dashed"; case .valid: "checkmark.circle"; case .warning: "exclamationmark.triangle"; case .error: "xmark.circle" }
    }
}
