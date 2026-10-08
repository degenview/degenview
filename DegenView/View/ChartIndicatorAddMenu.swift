import SwiftUI

/// The header's "+": faint on an empty chart, shown only while the header is hovered once there are
/// indicators, so it never competes with the chart's own controls.
struct ChartIndicatorAddMenu: View {
    @ObservedObject var viewModel: ChartViewModel
    /// The chart has no indicators, so the button is the only sign that they exist.
    let isEmpty: Bool
    let isHeaderHovered: Bool
    var onStyleChanged: () -> Void = {}
    let onOpenSettings: () -> Void

    @ObservedObject private var saved = SavedScriptsModel.shared
    @State private var hovering = false
    @State private var addError: String?

    private var restingOpacity: Double {
        if hovering { return 0.85 }
        if isEmpty { return isHeaderHovered ? 0.6 : 0.35 }
        return isHeaderHovered ? 0.5 : 0
    }

    var body: some View {
        Menu {
            builtIns
            scripts
            Divider()
            if !viewModel.usesLineChart {
                Button("Script Manager…") { WindowCoordinator.shared.openScriptManager(selecting: nil) }
            }
            Button("All Indicator Settings…", action: onOpenSettings)
        } label: {
            Image(systemName: "plus")
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(Color.primary.opacity(restingOpacity))
                .frame(width: 20, height: 20)
                .background(Circle().fill(Color.primary.opacity(hovering ? 0.10 : 0)))
                .contentShape(Circle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .onHover { hovering = $0 }
        .animation(.easeInOut(duration: 0.12), value: restingOpacity)
        .help("Add Indicator")
        .accessibilityLabel("Add Indicator")
        .task { await saved.loadIfNeeded() }
        .alert(
            "Couldn't add script", isPresented: Binding(get: { addError != nil }, set: { if !$0 { addError = nil } })
        ) {
            Button("OK") { addError = nil }
        } message: {
            Text(addError ?? "")
        }
    }

    @ViewBuilder private var builtIns: some View {
        let addable = viewModel.addableBuiltIns
        if !addable.isEmpty {
            Section("Built-in") {
                ForEach(addable, id: \.indicator) { entry in
                    Button {
                        entry.indicator.setOn(true, in: viewModel)
                        onStyleChanged()
                    } label: {
                        Label(entry.indicator.menuTitle, systemImage: entry.indicator.icon)
                    }
                    .disabled(entry.availability != .available)
                    .help(
                        { if case .unavailable(let reason) = entry.availability { return reason } else { return "" } }()
                    )
                }
            }
        }
    }

    /// Pine scripts do not draw on line charts (prediction markets), so they are not offered there.
    @ViewBuilder private var scripts: some View {
        if !viewModel.usesLineChart {
            if !saved.indicatorScripts.isEmpty {
                Section("Scripts") {
                    ForEach(saved.indicatorScripts) { script in
                        Button(script.name) { add(script) }
                    }
                }
            }
            if !saved.strategyScripts.isEmpty {
                Section("Strategies") {
                    ForEach(saved.strategyScripts) { script in
                        Button(script.name) { add(script) }
                    }
                }
            }
        }
    }

    private func add(_ script: LocalScript) {
        if let error = saved.add(script, to: viewModel) {
            addError = error
        } else {
            onStyleChanged()
        }
    }
}
