import SwiftUI

/// One legend row's display data, derived fresh from `scriptInstances`/`pineResults` — never
/// stored, so it cannot drift from the model.
struct PineLegendRowInfo: Identifiable {
    let id: UUID  // == ChartScriptInstance.id
    let scriptID: UUID  // the library script this instance runs
    let title: String  // the declaration title; inputs live in the settings popover
    let isVisible: Bool
    let hasError: Bool
}

extension ChartViewModel {
    var pineLegendRows: [PineLegendRowInfo] {
        scriptInstances.map { instance in
            let result = pineResults[instance.id]
            let title = result?.declaration?.title ?? "Script"
            let hasError = result?.diagnostics.contains { $0.severity == .error } ?? false
            return PineLegendRowInfo(
                id: instance.id, scriptID: instance.scriptID, title: title, isVisible: instance.isVisible,
                hasError: hasError)
        }
    }
}

/// TradingView-style applied-indicator legend, overlaid on the chart's upper-left corner.
/// Reads `ChartViewModel` state only; owns no persisted state of its own.
struct PineIndicatorLegend: View {
    @ObservedObject var viewModel: ChartViewModel
    /// Hands the legend's own `NSView` to the window-wide drawing-tool monitor, so a click on
    /// a row's icons is never swallowed as "begin drawing".
    var onLegendRegion: (NSView) -> Void = { _ in }
    /// Marks the chart's saved layout dirty — visibility/input/removal are persisted
    /// configuration, unlike the runtime output they affect.
    var onStyleChanged: () -> Void = {}

    @State private var legendHovering = false
    @State private var expandedOverride = false
    /// Whether the whole legend is minimized to just its header strip. Session-local — it's a
    /// view convenience, not persisted chart configuration.
    @State private var collapsed = true
    private static let collapseThreshold = 4
    private static let cornerRadius: CGFloat = 10

    var body: some View {
        let rows = viewModel.pineLegendRows
        if !rows.isEmpty {
            let showAll = legendHovering || expandedOverride || rows.count <= Self.collapseThreshold
            VStack(alignment: .leading, spacing: 0) {
                header(count: rows.count)
                if !collapsed {
                    Rectangle()
                        .fill(Color.primary.opacity(0.10))
                        .frame(height: 1)
                        .padding(.vertical, 4)
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(showAll ? rows : Array(rows.prefix(Self.collapseThreshold))) { row in
                            PineIndicatorLegendRow(viewModel: viewModel, info: row, onStyleChanged: onStyleChanged)
                        }
                        if !showAll {
                            Button {
                                expandedOverride = true
                            } label: {
                                Text("+\(rows.count - Self.collapseThreshold) more")
                                    .font(.caption2.weight(.medium))
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 3)
                                    .background(Color.primary.opacity(0.08), in: Capsule())
                            }
                            .buttonStyle(.plain)
                            .padding(.horizontal, 6)
                            .padding(.top, 2)
                        }
                    }
                    .transition(.opacity.combined(with: .move(edge: .top)))
                }
            }
            .padding(7)
            // The row/header Spacers push controls to the trailing edge of the legend's own
            // content — not of whatever width the chart overlay offers. Without this, the
            // legend would stretch to the full chart width.
            .fixedSize(horizontal: true, vertical: false)
            // Adaptive tint over the card's own material, so the legend follows light/dark and
            // stays only faintly distinct from the chart behind it.
            .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: Self.cornerRadius))
            .overlay(
                RoundedRectangle(cornerRadius: Self.cornerRadius)
                    .strokeBorder(Color.primary.opacity(0.10))
            )
            .shadow(color: .black.opacity(0.08), radius: 4, y: 1)
            .animation(.easeInOut(duration: 0.16), value: collapsed)
            .onHover { legendHovering = $0 }
            .background(PineLegendHitRegion(onResolve: onLegendRegion))
            .accessibilityElement(children: .contain)
        }
    }

    private func header(count: Int) -> some View {
        // HStack spacing also applies on both sides of the Spacer, so collapsed zeroes both.
        HStack(spacing: collapsed ? 0 : 6) {
            if !collapsed {
                Image(systemName: "chart.xyaxis.line")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            Text(collapsed ? "\(count)" : "Indicators")
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.primary)
            Spacer(minLength: collapsed ? 3 : 10)
            Button {
                withAnimation(.easeInOut(duration: 0.16)) { collapsed.toggle() }
            } label: {
                Image(systemName: "chevron.down")
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(collapsed ? -90 : 0))
                    .frame(width: 14, height: 14)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(collapsed ? "Expand indicator list" : "Collapse indicator list")
        }
        .padding(.leading, 4)
        .padding(.trailing, collapsed ? 0 : 4)
        .padding(.vertical, 1)
        .contentShape(Rectangle())
        .onTapGesture { withAnimation(.easeInOut(duration: 0.16)) { collapsed.toggle() } }
    }
}

private struct PineIndicatorLegendRow: View {
    @ObservedObject var viewModel: ChartViewModel
    let info: PineLegendRowInfo
    var onStyleChanged: () -> Void = {}
    @State private var rowHovering = false
    @State private var showingSettings = false
    @State private var settingsDismissedAt = Date.distantPast

    /// Controls stay visible while the settings popover is open even if the pointer has moved
    /// off the row into the popover itself — otherwise the popover's own anchor unmounts and
    /// SwiftUI dismisses it before the user can touch an input.
    private var controlsVisible: Bool { rowHovering || showingSettings }

    /// A transient popover closes on the mouse-down of any outside click, so by the time the name's
    /// button fires the popover is already gone and a plain toggle would reopen it. A press right
    /// after a dismissal is therefore the click that closed it, not a request to open it again.
    private func toggleSettings() {
        if showingSettings {
            showingSettings = false
        } else if Date().timeIntervalSince(settingsDismissedAt) > 0.3 {
            showingSettings = true
        }
    }

    var body: some View {
        HStack(spacing: 7) {
            // The name opens the same inputs popover as the gear, and closes it again.
            Button {
                toggleSettings()
            } label: {
                Text(info.title)
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(info.isVisible ? .primary : .secondary)
                    .lineLimit(1)
            }
            .buttonStyle(.plain)
            .help("Edit inputs")
            .accessibilityLabel("\(info.title) inputs")
            if info.hasError {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 10))
                    .foregroundStyle(.orange)
                    .accessibilityLabel("\(info.title) has a runtime error")
            }
            Spacer(minLength: 10)
            // Always laid out (never conditionally inserted/removed) so the row's height never
            // changes on hover, and so the inputs button's popover anchor never unmounts.
            control("eye", alt: info.isVisible ? nil : "eye.slash") {
                viewModel.setPineInstanceVisible(info.id, isVisible: !info.isVisible)
                onStyleChanged()
            }
            .accessibilityLabel(info.isVisible ? "Hide \(info.title)" : "Show \(info.title)")

            control("slider.horizontal.3") { showingSettings = true }
                .help("Edit inputs")
                .accessibilityLabel("\(info.title) settings")
                .popover(isPresented: $showingSettings) {
                    PineInstanceSettingsPopover(
                        viewModel: viewModel, instanceID: info.id, onStyleChanged: onStyleChanged)
                }

            control("curlybraces") { openInScriptManager() }
                .help("Edit script in Script Manager")
                .accessibilityLabel("Edit \(info.title) in Script Manager")

            control("xmark") {
                viewModel.removePineInstance(info.id)
                onStyleChanged()
            }
            .accessibilityLabel("Remove \(info.title)")
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .background(
            RoundedRectangle(cornerRadius: 5)
                .fill(Color.primary.opacity(rowHovering ? 0.08 : 0))
        )
        .contentShape(Rectangle())
        .onHover { hovering in
            withAnimation(.easeInOut(duration: 0.1)) { rowHovering = hovering }
        }
        .opacity(info.isVisible ? 1 : 0.55)
        .onChange(of: showingSettings) { _, showing in
            if !showing { settingsDismissedAt = Date() }
        }
        .accessibilityElement(children: .combine)
    }

    /// Opens the Script Manager on this instance's script, previewing the chart's own market
    /// (when the preview supports its source; otherwise it keeps its current one).
    private func openInScriptManager() {
        let market = PreviewMarket(ticker: viewModel.ticker, source: viewModel.source)
        WindowCoordinator.shared.openScriptManager(
            selecting: info.scriptID, market: PreviewMarket.isSupported(market.source) ? market : nil)
    }

    /// One fixed-size control icon — `alt` swaps the symbol (for the eye/eye.slash toggle)
    /// without changing the button's identity, so SwiftUI never recreates it mid-hover.
    @ViewBuilder
    private func control(_ systemName: String, alt: String? = nil, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: alt ?? systemName)
                .font(.system(size: 11, weight: .medium))
                .frame(width: 16, height: 16)
        }
        .buttonStyle(.plain)
        .foregroundStyle(.primary.opacity(0.85))
        .opacity(controlsVisible ? 1 : 0)
        .allowsHitTesting(controlsVisible)
        .accessibilityHidden(!controlsVisible)
    }
}

/// Non-interactive AppKit view stretched over the legend's own frame — same pattern as
/// `ZoomHitRegion`/`PlotHitRegion` elsewhere in the chart card. `hitTest` returns nil so
/// SwiftUI buttons inside the legend keep every click; its only job is handing its frame to
/// `ContentViewModel` so the drawing-tool monitor can exclude it.
private struct PineLegendHitRegion: NSViewRepresentable {
    let onResolve: (NSView) -> Void
    func makeNSView(context: Context) -> NSView {
        let view = PassthroughView()
        onResolve(view)
        return view
    }
    func updateNSView(_ nsView: NSView, context: Context) {}
    private final class PassthroughView: NSView {
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}
