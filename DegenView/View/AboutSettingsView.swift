import AppKit
import SwiftUI

/// The Settings ▸ About page: icon, name and version, build details, project links, copyright.
struct AboutSettingsView: View {
    private let info = AppInfo()
    @StateObject private var updates = UpdateCheckViewModel(currentVersion: AppInfo().version)
    @State private var copied = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                header
                UpdateStatusCard(model: updates, currentVersion: info.version)
                AboutGroup {
                    if let build = info.distinctBuild {
                        AboutRow(title: "Build", systemImage: "hammer.fill", tint: .gray) {
                            valueText(build)
                        }
                        AboutDivider()
                    }
                    AboutRow(title: "License", systemImage: "doc.text.fill", tint: .teal) {
                        valueText(AppInfo.licenseName)
                    }
                    AboutDivider()
                    AboutRow(title: "Build Info", systemImage: "info.circle.fill", tint: .indigo) {
                        copyButton
                    }
                }
                AboutGroup {
                    AboutLinkRow(
                        title: "GitHub",
                        hint: "Source code, issues and releases",
                        logo: "BrandLogo-github",
                        tile: Color(red: 0.094, green: 0.090, blue: 0.090),
                        destination: AppInfo.repositoryURL)
                    AboutDivider()
                    AboutLinkRow(
                        title: "Support DegenView",
                        hint: "Buy me a coffee — it keeps development going",
                        logo: "BrandLogo-buymeacoffee",
                        tile: .white,
                        destination: AppInfo.donationURL)
                }
                Text(info.copyright)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity)
            }
            .frame(maxWidth: 480)
            .padding(28)
            .frame(maxWidth: .infinity)
        }
        .task {
            if updates.status == .idle { await updates.check() }
        }
    }

    private var header: some View {
        HStack(spacing: 16) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .interpolation(.high)
                .frame(width: 72, height: 72)
                .shadow(color: .black.opacity(0.22), radius: 7, y: 3)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(info.name)
                    .font(.system(size: 28, weight: .bold, design: .rounded))
                Text(info.headerSubtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
    }

    private func valueText(_ value: String) -> some View {
        Text(value)
            .font(.subheadline)
            .monospacedDigit()
            .foregroundStyle(.secondary)
            .textSelection(.enabled)
    }

    private var copyButton: some View {
        Button(action: copySummary) {
            Label(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc")
                .font(.caption.weight(.medium))
                .contentTransition(.symbolEffect(.replace))
        }
        .buttonStyle(.borderless)
        .foregroundStyle(copied ? AnyShapeStyle(.green) : AnyShapeStyle(.tint))
        .help("Copy the version and build date for a bug report")
    }

    private func copySummary() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(info.clipboardSummary, forType: .string)
        withAnimation { copied = true }
        Task {
            try? await Task.sleep(for: .seconds(1.6))
            withAnimation { copied = false }
        }
    }
}
