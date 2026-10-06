import SwiftUI

/// The tab's one saved-layout control: the layout's name, a chevron, and — when nothing will save
/// the current changes — an "(unsaved)" tag after the name. Everything else lives in the menu it opens.
struct SavedLayoutToolbarButton: View {
    @ObservedObject var layout: SavedLayoutController
    let onOpenAll: () -> Void

    var body: some View {
        Menu {
            Button("Save layout") { layout.requestSave() }
                .keyboardShortcut("s", modifiers: .command)

            Toggle("Autosave", isOn: autosaveBinding)
                .disabled(layout.isUnnamed)

            Divider()

            Button("Make a copy") { layout.prompt = .copy }
                .disabled(layout.isUnnamed)
            Button("Rename") { layout.prompt = .rename }
            Button("Create new layout") { layout.createNewLayout() }

            let recents = layout.recentViews
            if !recents.isEmpty {
                Divider()
                Section("Recently used") {
                    ForEach(recents) { view in
                        Button(view.name) { layout.requestOpen(view) }
                    }
                }
            }

            Divider()
            Button("Open layout…", action: onOpenAll)
        } label: {
            label
        }
        .menuIndicator(.hidden)
        .help(helpText)
        .accessibilityLabel(accessibilityText)
    }

    private var label: some View {
        HStack(spacing: 4) {
            HStack(spacing: 4) {
                Text(layout.displayName)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: SavedLayout.nameMaxWidth, alignment: .leading)
                    .fixedSize(horizontal: true, vertical: false)
                if layout.showsSaveAffordance {
                    Text("(unsaved)")
                        .foregroundStyle(.secondary)
                }
            }
            Image(systemName: "chevron.down")
                .font(.caption2.weight(.semibold))
                .accessibilityHidden(true)
        }
        .padding(.horizontal, 6)
    }

    private var autosaveBinding: Binding<Bool> {
        Binding(get: { layout.autosaveEnabled }, set: { layout.setAutosave($0) })
    }

    private var helpText: String {
        if layout.autosaveFailed { return "\(layout.displayName) — Autosave could not save; choose Save layout" }
        return layout.isDirty && layout.showsSaveAffordance
            ? "\(layout.displayName) — unsaved changes" : layout.displayName
    }

    private var accessibilityText: String {
        layout.showsSaveAffordance
            ? "Layout: \(layout.displayName), unsaved changes" : "Layout: \(layout.displayName)"
    }
}
