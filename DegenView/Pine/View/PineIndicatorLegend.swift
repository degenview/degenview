import SwiftUI

/// One legend row's display data, derived fresh from `scriptInstances`/`pineResults` — never
/// stored, so it cannot drift from the model.
struct PineLegendRowInfo: Identifiable {
    let id: UUID  // == ChartScriptInstance.id
    let title: String  // "EMA 20" — declaration title + compact input summary
    let isVisible: Bool
    let hasError: Bool
}

extension ChartViewModel {
    var pineLegendRows: [PineLegendRowInfo] {
        scriptInstances.map { instance in
            let result = pineResults[instance.id]
            let base = result?.declaration?.title ?? "Script"
            let summary = result.map { $0.inputSchema.pineCompactSummary(values: instance.inputs) } ?? ""
            let title = summary.isEmpty ? base : "\(base) \(summary)"
            let hasError = result?.diagnostics.contains { $0.severity == .error } ?? false
            return PineLegendRowInfo(id: instance.id, title: title, isVisible: instance.isVisible, hasError: hasError)
        }
    }
}

extension PineInputSchema {
    /// A short "<v1> <v2> …" summary of this schema's int/float inputs, current-value-first —
    /// the same compact style TradingView's legend uses ("EMA 20", "Supertrend 10 3"). Other
    /// input kinds are omitted; they rarely fit a one-line row.
    func pineCompactSummary(values: [String: PineInputValue]) -> String {
        inputs.compactMap { input -> String? in
            switch values[input.id] ?? input.defaultValue {
            case .int(let v): return "\(v)"
            case .float(let v): return String(format: "%g", v)
            default: return nil
            }
        }.joined(separator: " ")
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
    private static let collapseThreshold = 4

    var body: some View {
        let rows = viewModel.pineLegendRows
        if !rows.isEmpty {
            let showAll = legendHovering || expandedOverride || rows.count <= Self.collapseThreshold
            VStack(alignment: .leading, spacing: 1) {
                ForEach(showAll ? rows : Array(rows.prefix(Self.collapseThreshold))) { row in
                    PineIndicatorLegendRow(viewModel: viewModel, info: row, onStyleChanged: onStyleChanged)
                }
                if !showAll {
                    Button("+\(rows.count - Self.collapseThreshold) more") { expandedOverride = true }
                        .buttonStyle(.plain)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                }
            }
            .padding(5)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
            .onHover { legendHovering = $0 }
            .background(PineLegendHitRegion(onResolve: onLegendRegion))
            .accessibilityElement(children: .contain)
        }
    }
}

private struct PineIndicatorLegendRow: View {
    @ObservedObject var viewModel: ChartViewModel
    let info: PineLegendRowInfo
    var onStyleChanged: () -> Void = {}
    @State private var rowHovering = false
    @State private var showingSettings = false

    var body: some View {
        HStack(spacing: 6) {
            Text(info.title)
                .font(.caption2.weight(.medium))
                .foregroundStyle(info.isVisible ? .primary : .secondary)
                .lineLimit(1)
            if info.hasError {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.caption2)
                    .foregroundStyle(.orange)
                    .accessibilityLabel("\(info.title) has a runtime error")
            }
            if rowHovering {
                Button {
                    viewModel.setPineInstanceVisible(info.id, isVisible: !info.isVisible)
                    onStyleChanged()
                } label: {
                    Image(systemName: info.isVisible ? "eye" : "eye.slash")
                }
                .buttonStyle(.plain)
                .accessibilityLabel(info.isVisible ? "Hide \(info.title)" : "Show \(info.title)")

                Button { showingSettings = true } label: { Image(systemName: "gearshape") }
                    .buttonStyle(.plain)
                    .accessibilityLabel("\(info.title) settings")
                    .popover(isPresented: $showingSettings) {
                        PineInstanceSettingsPopover(
                            viewModel: viewModel, instanceID: info.id, onStyleChanged: onStyleChanged)
                    }

                Button {
                    viewModel.removePineInstance(info.id)
                    onStyleChanged()
                } label: { Image(systemName: "xmark") }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Remove \(info.title)")
            }
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .onHover { rowHovering = $0 }
        .opacity(info.isVisible ? 1 : 0.55)
        .accessibilityElement(children: .combine)
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
