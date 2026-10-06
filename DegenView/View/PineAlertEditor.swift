import SwiftUI

/// Creates a notification subscription for the alerts one of a chart's applied indicators raises.
struct PineAlertEditor: View {
    @ObservedObject var viewModel: ChartViewModel
    /// The applied indicator (`ChartScriptInstance.id`) the alert is for.
    let instanceID: UUID
    @StateObject private var store = PineAlertStore.shared
    @Environment(\.dismiss) private var dismiss
    @State private var note = ""
    @State private var scriptName = ""
    @State private var callSites: [PineAlertCallSite] = []
    /// False until the applied script has been compiled; nothing is blocked while it is unknown.
    @State private var loaded = false

    private static let inlineCallLimit = 4

    private var dataset: PineDatasetKey { viewModel.pineAlertDataset }

    private var instance: ChartScriptInstance? { viewModel.scriptInstances.first { $0.id == instanceID } }

    private var duplicateExists: Bool {
        store.subscriptions(forChart: viewModel.chartID).contains {
            $0.isActive && $0.instanceID == instanceID && $0.watches(dataset)
        }
    }

    /// Why the alert cannot be created yet, if it cannot.
    private var blocker: (title: String, detail: String)? {
        guard loaded else { return nil }
        if instance == nil {
            return ("Indicator removed", "This indicator is no longer applied to the chart.")
        }
        if viewModel.pineInstanceSourceHashes[instanceID] == nil {
            return ("Script not loaded", "The indicator's script hasn't loaded yet. Try again in a moment.")
        }
        if callSites.isEmpty {
            return ("No alerts in this script", "The script has no alert() or alertcondition() calls.")
        }
        if duplicateExists {
            return ("Already armed", "This indicator already has an active alert on this chart.")
        }
        return nil
    }

    private var canCreate: Bool { loaded && blocker == nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SheetHeader(
                title: "Create Script Alert",
                subtitle: "\(viewModel.title) · \(viewModel.source.displayName)"
            ) {
                ChartIconView(viewModel: viewModel, size: 40)
                    .padding(.trailing, 3)
                    .padding(.bottom, 3)
            }
            scriptCard
            triggersSection
            noteField
            if let blocker {
                NoticeCard(
                    systemImage: "exclamationmark.triangle.fill", tint: .orange,
                    title: blocker.title, detail: blocker.detail)
            }
            footnote
            footer
        }
        .padding(24)
        .frame(width: 480)
        .task { await load() }
    }

    // MARK: - Sections

    private var scriptCard: some View {
        HStack(spacing: 12) {
            Image(systemName: "curlybraces")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 36, height: 36)
                .background(Color.accentColor.opacity(0.14), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 5) {
                Text(displayedScriptName)
                    .font(.headline)
                    .lineLimit(1)
                    .truncationMode(.tail)
                HStack(spacing: 6) {
                    pill(viewModel.title, systemImage: "chart.line.uptrend.xyaxis")
                    pill(dataset.timeframe, systemImage: "clock")
                }
            }
            Spacer(minLength: 8)
            if !loaded {
                SettingsStatusBadge(text: "Checking…", tone: .neutral)
            } else if blocker == nil {
                SettingsStatusBadge(text: "Ready", tone: .good)
            } else {
                SettingsStatusBadge(text: "Can't arm", tone: .warning)
            }
        }
        .padding(14)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(.separator.opacity(0.5)))
        .accessibilityElement(children: .combine)
    }

    private var triggersSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Alerts in this script").font(.subheadline.weight(.semibold))
            if callSites.isEmpty {
                Text(loaded ? "No alert calls" : "Reading the script…")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            } else if callSites.count > Self.inlineCallLimit {
                ScrollView { callRows }.frame(maxHeight: 150)
            } else {
                callRows
            }
        }
    }

    private var callRows: some View {
        VStack(spacing: 6) {
            ForEach(Array(callSites.enumerated()), id: \.offset) { _, site in
                callRow(site)
            }
        }
    }

    private func callRow(_ site: PineAlertCallSite) -> some View {
        HStack(spacing: 10) {
            Image(systemName: site.isCondition ? "bell.badge" : "bell.fill")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 24, height: 24)
                .background(Color.accentColor.opacity(0.14), in: Circle())
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(site.title ?? site.message ?? "Message set by the script")
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                    .truncationMode(.tail)
                if site.title != nil, let message = site.message {
                    Text(message).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
                }
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 3) {
                Text(site.frequency?.displayName ?? "Set by script")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(Color.primary.opacity(0.06), in: Capsule())
                Text(verbatim: "Line \(site.range.start.line)")
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.4)))
        .help(site.isCondition ? "alertcondition()" : "alert()")
        .accessibilityElement(children: .combine)
    }

    private var noteField: some View {
        HStack(spacing: 8) {
            Image(systemName: "text.alignleft").foregroundStyle(.secondary)
            TextField("Add a note (optional)", text: $note).textFieldStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.6)))
    }

    private var footnote: some View {
        Label {
            Text(
                "Fires only while this chart shows this symbol and timeframe and its live feed is running. "
                    + "Editing the script pauses the alert until you re-arm it."
            )
            .fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: "info.circle")
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Text(summary)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .controlSize(.large)
            Button("Create Alert") {
                PineAlertCoordinator.shared.subscribe(
                    viewModel, instanceID: instanceID, scriptName: scriptName, note: note)
                dismiss()
            }
            .buttonStyle(.borderedProminent)
            .keyboardShortcut(.defaultAction)
            .controlSize(.large)
            .disabled(!canCreate)
        }
        .padding(.top, 4)
    }

    // MARK: - Pieces

    private func pill(_ text: String, systemImage: String) -> some View {
        HStack(spacing: 5) {
            Image(systemName: systemImage).font(.system(size: 10, weight: .bold))
            Text(text).font(.caption.weight(.medium)).lineLimit(1)
        }
        .foregroundStyle(.secondary)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Color.primary.opacity(0.06), in: Capsule())
        .fixedSize()
    }

    // MARK: - Values

    private var displayedScriptName: String {
        if !scriptName.isEmpty { return scriptName }
        return loaded ? "Untitled script" : "Loading…"
    }

    /// "Fires on live bars of BTC/USDT · 1h", shown beside the Create button.
    private var summary: String {
        canCreate ? "Fires on live bars of \(viewModel.title) · \(dataset.timeframe)" : "Nothing to arm yet."
    }

    // MARK: - Actions

    private func load() async {
        defer { loaded = true }
        guard let source = viewModel.pineInstanceResolvedSource(instanceID), !source.isEmpty else { return }
        let program = PineCompiler.compile(source: source, libraries: PineLibraryRegistry.shared)
        callSites = program.alertCallSites
        if let id = instance?.scriptID,
            let saved = try? await ScriptStore.shared.script(id: id)
        {
            scriptName = saved.name
        } else {
            scriptName = program.declaration.title
        }
    }
}
