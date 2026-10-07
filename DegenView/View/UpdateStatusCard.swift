import SwiftUI

/// The About page's stand-alone update panel: a large state icon, a headline that says where the app
/// stands, a supporting line, and the one action that fits ("Check Now" or "Download").
struct UpdateStatusCard: View {
    @ObservedObject var model: UpdateCheckViewModel
    let currentVersion: String

    var body: some View {
        HStack(spacing: 14) {
            badge
            VStack(alignment: .leading, spacing: 3) {
                Text(headline).font(.headline)
                Text(detail).font(.subheadline).foregroundStyle(.secondary)
            }
            Spacer(minLength: 12)
            action
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(0.10), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(tint.opacity(0.28), lineWidth: 1)
        }
        .animation(.easeInOut(duration: 0.25), value: model.status)
        .accessibilityElement(children: .combine)
    }

    // MARK: - Content

    private var badge: some View {
        ZStack {
            Circle().fill(tint.gradient)
            if model.isChecking {
                ProgressView().controlSize(.small).tint(.white)
            } else {
                Image(systemName: symbol)
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(.white)
                    .contentTransition(.symbolEffect(.replace))
            }
        }
        .frame(width: 44, height: 44)
    }

    @ViewBuilder
    private var action: some View {
        switch model.status {
        case .available(_, let url):
            Link(destination: url) {
                Label("Download", systemImage: "arrow.down.to.line")
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
        default:
            Button("Check Now") { Task { await model.check() } }
                .buttonStyle(.bordered)
                .controlSize(.large)
                .disabled(model.isChecking)
        }
    }

    private var headline: String {
        switch model.status {
        case .idle: "Check for updates"
        case .checking: "Checking for updates…"
        case .upToDate: "DegenView is up to date"
        case .available(let version, _): "Version \(version) is available"
        case .failed: "Couldn't check for updates"
        }
    }

    private var detail: String {
        switch model.status {
        case .idle: "You're on version \(currentVersion)."
        case .checking: "You're on version \(currentVersion)."
        case .upToDate(let date):
            "You're on the latest version, \(currentVersion). Checked \(date.formatted(.relative(presentation: .named)))."
        case .available: "You're on version \(currentVersion)."
        case .failed(let message): message
        }
    }

    private var symbol: String {
        switch model.status {
        case .idle: "arrow.triangle.2.circlepath"
        case .checking: "arrow.triangle.2.circlepath"
        case .upToDate: "checkmark"
        case .available: "arrow.down"
        case .failed: "exclamationmark"
        }
    }

    private var tint: Color {
        switch model.status {
        case .idle, .checking: .gray
        case .upToDate: .green
        case .available: .blue
        case .failed: .orange
        }
    }
}
