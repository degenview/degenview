import SwiftUI

/// The alert control of a script row: creating one (possibly blocked, with the reason) or opening the
/// existing one.
enum ScriptAlertControl: Equatable {
    case create(blockedReason: String?)
    case existing(statusText: String, tone: SettingsStatusBadge.Tone)
}

/// One applied script in the chart's Scripts list: what it is, how it is doing, and the four things
/// you can do to it. Pure — everything comes in as values and closures — so it previews on its own.
struct ScriptInstanceRow<InputsPopover: View>: View {
    let title: String
    let kind: ScriptType
    /// Drawn on the price scale (true) or in its own pane (false); nil while unknown.
    let isOverlay: Bool?
    let state: PineInstanceResult.State
    /// The first error diagnostic, which outranks the state text.
    let problem: String?
    let isVisible: Bool
    let alert: ScriptAlertControl
    @Binding var isEditingInputs: Bool
    var onToggleVisible: () -> Void
    var onAlert: () -> Void
    var onRemove: () -> Void
    var onMoveUp: (() -> Void)?
    var onMoveDown: (() -> Void)?
    @ViewBuilder var inputsPopover: () -> InputsPopover

    private var failure: String? {
        if let problem { return problem }
        if case .failed(let text) = state { return text }
        return nil
    }

    private var badgeTint: Color { failure == nil ? .accentColor : .orange }

    private var kindIcon: String {
        switch kind {
        case .strategy: "arrow.left.arrow.right"
        case .indicator: "chart.xyaxis.line"
        case .library: "curlybraces"
        }
    }

    private var meta: String {
        var parts = [kind.displayName]
        if kind == .indicator, let isOverlay { parts.append(isOverlay ? "On chart" : "Own pane") }
        if !isVisible { parts.append("Hidden") }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: failure == nil ? kindIcon : "exclamationmark.triangle.fill")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(badgeTint)
                .frame(width: 30, height: 30)
                .background(badgeTint.opacity(0.12), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                    .truncationMode(.tail)
                detail
            }
            .opacity(isVisible ? 1 : 0.55)

            Spacer(minLength: 12)

            if case .existing(let text, let tone) = alert {
                SettingsStatusBadge(text: text, tone: tone)
                    .lineLimit(1)
                    .help("Alert: \(text)")
            }
            controls
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .contentShape(Rectangle())
        .contextMenu { menu }
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder private var detail: some View {
        if let failure {
            Text(failure)
                .font(.caption)
                .foregroundStyle(.orange)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
                .help(failure)
        } else if state == .evaluating {
            HStack(spacing: 6) {
                ProgressView().controlSize(.mini)
                Text("Evaluating…").font(.caption).foregroundStyle(.secondary)
            }
        } else {
            Text(meta).font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
    }

    private var controls: some View {
        HStack(spacing: 2) {
            SettingsIconButton(
                systemImage: isVisible ? "eye" : "eye.slash",
                label: isVisible ? "Hide \(title) on the chart" : "Show \(title) on the chart",
                action: onToggleVisible)
            alertButton
            SettingsIconButton(
                systemImage: "slider.horizontal.3", label: "Edit \(title) settings",
                action: { isEditingInputs = true }
            )
            .popover(isPresented: $isEditingInputs) { inputsPopover() }
            SettingsIconButton(
                systemImage: "xmark", label: "Remove \(title) from this chart", isDestructive: true,
                action: onRemove)
        }
    }

    @ViewBuilder private var alertButton: some View {
        switch alert {
        case .existing(let text, _):
            SettingsIconButton(
                systemImage: "bell.badge", label: "View alert for \(title) — \(text)",
                tint: .accentColor, action: onAlert)
        case .create(let blockedReason):
            SettingsIconButton(
                systemImage: "bell", label: blockedReason ?? "Notify me when \(title) raises alert() on a live bar",
                action: onAlert
            )
            .disabled(blockedReason != nil)
        }
    }

    @ViewBuilder private var menu: some View {
        Button(isVisible ? "Hide on Chart" : "Show on Chart", action: onToggleVisible)
        Button("Settings…") { isEditingInputs = true }
        switch alert {
        case .existing: Button("View Alert…", action: onAlert)
        case .create(let blockedReason): Button("Create Alert…", action: onAlert).disabled(blockedReason != nil)
        }
        Divider()
        Button("Move Up") { onMoveUp?() }.disabled(onMoveUp == nil)
        Button("Move Down") { onMoveDown?() }.disabled(onMoveDown == nil)
        Divider()
        Button("Remove from Chart", role: .destructive, action: onRemove)
    }
}

#Preview("Rows") {
    @Previewable @State var editing = false
    func row(
        _ title: String, kind: ScriptType = .indicator, state: PineInstanceResult.State = .ready,
        problem: String? = nil, visible: Bool = true, alert: ScriptAlertControl = .create(blockedReason: nil)
    ) -> some View {
        ScriptInstanceRow(
            title: title, kind: kind, isOverlay: true, state: state, problem: problem, isVisible: visible,
            alert: alert, isEditingInputs: $editing, onToggleVisible: {}, onAlert: {}, onRemove: {},
            onMoveUp: {}, onMoveDown: {}
        ) { EmptyView() }
    }
    return VStack(spacing: 0) {
        row("Volume Profile", alert: .existing(statusText: "Active", tone: .good))
        Divider()
        row("RSI Divergence", visible: false)
        Divider()
        row("Mean Reversion", kind: .strategy, alert: .create(blockedReason: "No alert() calls."))
        Divider()
        row("Broken Script", problem: "Undefined variable 'foo' at line 12")
        Divider()
        row("Loading", state: .evaluating, alert: .create(blockedReason: "Still loading this script…"))
    }
    .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 12))
    .frame(width: 640)
    .padding()
}
