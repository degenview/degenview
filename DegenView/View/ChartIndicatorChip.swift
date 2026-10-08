import SwiftUI

/// One applied indicator in the card header: a quiet name capsule that reveals its eye, settings and
/// remove controls while hovered. Name-only at rest so a full strip stays out of the way.
struct ChartIndicatorChip: View {
    @ObservedObject var viewModel: ChartViewModel
    let item: ChartIndicatorItem
    /// Keeps the controls on, for rows of the overflow list where there is no hover to reveal them.
    var alwaysShowControls = false
    var onStyleChanged: () -> Void = {}
    let onRemove: () -> Void

    @State private var hovering = false
    @State private var showingSettings = false
    @State private var settingsDismissedAt = Date.distantPast

    /// Controls stay up while the settings popover is open even if the pointer has moved into it —
    /// otherwise the chip would shrink, unmount the popover's anchor and dismiss it mid-edit.
    private var controlsVisible: Bool { alwaysShowControls || hovering || showingSettings }
    private var scriptInstanceID: UUID? {
        if case .script(let id) = item.kind { return id }
        return nil
    }

    /// A transient popover closes on the mouse-down of any outside click, so by the time the name's
    /// button fires it is already gone and a plain toggle would reopen it. A press right after a
    /// dismissal is the click that closed it, not a request to open it again.
    private func toggleSettings() {
        if showingSettings {
            showingSettings = false
        } else if Date().timeIntervalSince(settingsDismissedAt) > 0.3 {
            showingSettings = true
        }
    }

    var body: some View {
        HStack(spacing: 4) {
            name
            if item.hasError {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 9))
                    .foregroundStyle(.orange)
                    .accessibilityLabel("\(item.title) has a runtime error")
            }
            if controlsVisible {
                HStack(spacing: 1) {
                    control(item.isVisible ? "eye" : "eye.slash", label: item.isVisible ? "Hide" : "Show") {
                        toggleVisibility()
                    }
                    if item.hasSettings {
                        control("slider.horizontal.3", label: "Settings") { toggleSettings() }
                    }
                    control("xmark", label: "Remove", action: onRemove)
                }
                .transition(.opacity.combined(with: .scale(scale: 0.8, anchor: .leading)))
            }
        }
        .padding(.horizontal, 8)
        .frame(height: 20)
        .background(Capsule().fill(Color.primary.opacity(hovering || showingSettings ? 0.10 : 0.06)))
        .opacity(item.isVisible ? 1 : 0.55)
        .contentShape(Capsule())
        .onHover { hovering = $0 }
        .animation(.easeInOut(duration: 0.12), value: controlsVisible)
        .popover(isPresented: $showingSettings) { settingsContent }
        .onChange(of: showingSettings) { _, showing in
            if !showing { settingsDismissedAt = Date() }
        }
        .contextMenu { contextMenu }
        .accessibilityElement(children: .contain)
    }

    /// Scripts keep their instance and built-ins their flag; either way the indicator stays applied.
    private func toggleVisibility() {
        switch item.kind {
        case .script(let id): viewModel.setPineInstanceVisible(id, isVisible: !item.isVisible)
        case .builtIn(let indicator): indicator.setHidden(item.isVisible, in: viewModel)
        }
        onStyleChanged()
    }

    @ViewBuilder private var name: some View {
        let text = Text(item.title)
            .font(.caption2.weight(.medium))
            .foregroundStyle(item.isVisible ? Color.primary.opacity(0.7) : Color.secondary)
            .lineLimit(1)
            .frame(maxWidth: 110)
            .fixedSize(horizontal: true, vertical: false)
        if item.hasSettings {
            Button(action: toggleSettings) { text }
                .buttonStyle(.plain)
                .help("Edit \(item.title)")
                .accessibilityLabel("\(item.title) settings")
        } else {
            text
        }
    }

    @ViewBuilder private var settingsContent: some View {
        switch item.kind {
        case .script(let id):
            PineInstanceSettingsPopover(viewModel: viewModel, instanceID: id, onStyleChanged: onStyleChanged)
        case .builtIn(.ema):
            EMAPeriodPopover(viewModel: viewModel, onStyleChanged: onStyleChanged)
        case .builtIn:
            EmptyView()
        }
    }

    @ViewBuilder private var contextMenu: some View {
        Button(item.isVisible ? "Hide" : "Show", action: toggleVisibility)
        if item.hasSettings {
            Button("Settings…") { showingSettings = true }
        }
        if let id = scriptInstanceID {
            Button("Edit in Script Manager") { openInScriptManager(instanceID: id) }
        }
        Divider()
        Button("Remove", role: .destructive, action: onRemove)
    }

    /// Opens the Script Manager on this instance's script, previewing the chart's own market
    /// (when the preview supports its source; otherwise it keeps its current one).
    private func openInScriptManager(instanceID: UUID) {
        guard let scriptID = viewModel.scriptInstances.first(where: { $0.id == instanceID })?.scriptID else { return }
        let market = PreviewMarket(ticker: viewModel.ticker, source: viewModel.source)
        WindowCoordinator.shared.openScriptManager(
            selecting: scriptID, market: PreviewMarket.isSupported(market.source) ? market : nil)
    }

    private func control(_ systemName: String, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 10, weight: .medium))
                .frame(width: 16, height: 16)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(Color.primary.opacity(0.85))
        .help("\(label) \(item.title)")
        .accessibilityLabel("\(label) \(item.title)")
    }
}
