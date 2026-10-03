import SwiftUI

struct ScriptManagerView: View {
    @StateObject private var model = ScriptManagerViewModel()
    /// Outside the per-script workspace, so the market and its candles survive a change of script.
    @StateObject private var preview = ScriptPreviewViewModel()
    @State private var pendingDelete: LocalScript?
    @State private var showNewScript = false
    @AppStorage("appTheme") private var appTheme: AppTheme = .system
    @AppStorage("scriptManager.sidebarVisible") private var sidebarVisible = true
    @AppStorage("scriptManager.sidebarWidth") private var sidebarWidth = 0.0
    @AppStorage("scriptManager.chartVisible") private var chartVisible = true
    @AppStorage("scriptManager.chartPosition") private var chartPosition: ChartPosition = .left

    var body: some View {
        SplitContainer(
            axis: .horizontal, secondaryFirst: true, secondaryLength: $sidebarWidth,
            isSecondaryVisible: sidebarVisible, minPrimary: 420, minSecondary: 200, maxSecondary: 320,
            defaultLength: 240
        ) {
            detail
        } secondary: {
            sidebar
        }
        // The toolbar keeps AppKit's unified titlebar row alive for this native tab. The chart
        // tab's controls must disappear here, but removing toolbar content entirely collapses the
        // titlebar and makes the window jump vertically.
        .toolbar {
            AppToolbar()
            ToolbarItem(placement: .navigation) {
                Button {
                    withAnimation(.snappy) { sidebarVisible.toggle() }
                } label: {
                    Label("Toggle Sidebar", systemImage: "sidebar.left")
                }
                .keyboardShortcut("s", modifiers: [.control, .command])
                .help(sidebarVisible ? "Hide the script list" : "Show the script list")
            }
            ToolbarItemGroup(placement: .primaryAction) {
                Menu {
                    Picker("Chart Position", selection: $chartPosition) {
                        ForEach(ChartPosition.allCases) { position in
                            Label(position.title, systemImage: position.symbol).tag(position)
                        }
                    }
                    .pickerStyle(.inline)
                } label: {
                    Label("Chart Position", systemImage: chartPosition.symbol)
                }
                .help("Where the preview chart sits next to the code")

                Button {
                    withAnimation(.snappy) { chartVisible.toggle() }
                } label: {
                    Label(
                        "Toggle Chart",
                        systemImage: chartVisible ? "chart.xyaxis.line" : "chart.line.flattrend.xyaxis")
                }
                .keyboardShortcut("p", modifiers: [.option, .command])
                .help(chartVisible ? "Hide the preview chart" : "Show the preview chart")
            }
        }
        .task {
            model.load()
            if let market = WindowCoordinator.shared.takePendingScriptManagerMarket() {
                preview.selectMarket(market)
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .localScriptsDidChange)) { _ in model.load() }
        .onReceive(NotificationCenter.default.publisher(for: .selectPreviewMarketInManager)) { note in
            _ = WindowCoordinator.shared.takePendingScriptManagerMarket()
            if let market = note.object as? PreviewMarket { preview.selectMarket(market) }
        }
        .onReceive(NotificationCenter.default.publisher(for: .selectScriptInManager)) { note in
            if let id = note.object as? UUID { model.select(id) }
        }
        .onChange(of: model.scripts) { _, scripts in
            if model.hasLoaded { preview.prune(keeping: Set(scripts.map(\.id))) }
        }
        .background(WindowAccessor { WindowCoordinator.shared.registerAuxiliaryTab($0) })
        .alert("Script Manager", isPresented: .constant(model.errorMessage != nil)) {
            Button("OK") { model.errorMessage = nil }
        } message: {
            Text(model.errorMessage ?? "")
        }
        .sheet(isPresented: $showNewScript) {
            ScriptNameSheet(
                problemFor: { model.nameProblem(for: $0) },
                onCreate: { name in
                    showNewScript = false
                    model.create(named: name)
                },
                onCancel: { showNewScript = false })
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
        .preferredColorScheme(appTheme.colorScheme)
    }
}

extension ScriptManagerView {
    fileprivate var sidebar: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search name or source", text: $model.query)
                    .textFieldStyle(.plain)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            .padding(8)

            List(selection: $model.selection) {
                ForEach(model.groups) { group in
                    Section(isExpanded: model.isExpanded(group.id)) {
                        ForEach(group.rows) { row in
                            ScriptRow(
                                script: row.script, rowID: row.id, model: model,
                                onDelete: { pendingDelete = $0 }
                            )
                            .tag(row.script.id)
                        }
                    } header: {
                        Text(group.title)
                    }
                }
            }
            .listStyle(.sidebar)
            .frame(maxHeight: .infinity)
            // An inset, not an overlay: the last rows can still scroll clear of the button.
            .safeAreaInset(edge: .bottom, spacing: 0) {
                HStack {
                    Spacer(minLength: 0)
                    newScriptButton
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
            }
        }
    }

    fileprivate var newScriptButton: some View {
        Button {
            showNewScript = true
        } label: {
            Label("New Script", systemImage: "plus")
                .font(.system(size: 12, weight: .semibold))
                .padding(.horizontal, 4)
        }
        .buttonStyle(.borderedProminent)
        .buttonBorderShape(.capsule)
        .controlSize(.regular)
        .shadow(color: .black.opacity(0.18), radius: 4, y: 1)
        .help("Create a new script")
    }

    @ViewBuilder fileprivate var detail: some View {
        if let id = model.selection {
            ScriptWorkspaceView(
                scriptID: id, type: model.scripts.first { $0.id == id }?.type ?? .indicator, preview: preview
            )
            .id(id)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if model.hasLoaded && model.scripts.isEmpty {
            VStack(spacing: 12) {
                Text("No scripts yet")
                    .font(.headline)
                Text("Write a Pine Script indicator or strategy and run it on your charts.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                Button("Create Your First Script", systemImage: "plus") { showNewScript = true }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                HStack(spacing: 16) {
                    Link(
                        "Pine Script documentation",
                        destination: URL(string: "https://www.tradingview.com/pine-script-docs/")!)
                    Link(
                        "Community scripts",
                        destination: URL(string: "https://www.tradingview.com/scripts/")!)
                }
            }
            .padding()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            Text("Select a script")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
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

    private var nameIsValid: Bool { model.nameProblem(for: draftName, excluding: script.id) == nil }

    var body: some View {
        HStack {
            if model.renamingRowID == rowID {
                TextField("Name", text: $draftName)
                    .textFieldStyle(.plain)
                    .foregroundStyle(nameIsValid ? Color.primary : Color.red)
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
            Button("Rename") { model.renamingRowID = rowID }
            Divider()
            Button("Show in Finder") { model.showInFinder(script) }
            Button("Export…") { model.export(script) }
            Divider()
            Button("Delete", role: .destructive) { onDelete(script) }
        }
    }
}
