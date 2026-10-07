import SwiftUI

/// The webhook's secret: a masked field, a "Use secret…" menu that puts `{{secret}}` where the user
/// wants it, and a one-line status that says whether it is used yet. Keeps people from having to know
/// the placeholder syntax.
struct WebhookSecretSection: View {
    @Binding var secret: String
    @Binding var url: String
    @Binding var headers: [WebhookHeader]
    /// Opened when a choice lands in the headers.
    @Binding var headersExpanded: Bool

    @FocusState private var isFocused: Bool
    @State private var isRevealed = false

    private var urlUsesSecret: Bool { WebhookSecretPlaceholder.contains(url) }
    private var headerUseCount: Int { headers.filter { WebhookSecretPlaceholder.contains($0.value) }.count }
    private var isUsed: Bool { urlUsesSecret || headerUseCount > 0 }
    private var problem: WebhookSecretError? { secret.isEmpty ? nil : WebhookHeaderPolicy.validate(secret: secret) }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text("Secret").font(.subheadline.weight(.semibold))
                Text("Optional").font(.caption).foregroundStyle(.tertiary)
                Spacer(minLength: 8)
                useMenu
            }

            HStack(spacing: 8) {
                Image(systemName: "lock.fill").foregroundStyle(.secondary).font(.system(size: 12))
                Group {
                    if isRevealed {
                        TextField("API token or key", text: $secret)
                    } else {
                        SecureField("API token or key", text: $secret)
                    }
                }
                .textFieldStyle(.plain)
                .font(isRevealed ? .body.monospaced() : .body)
                .autocorrectionDisabled()
                .focused($isFocused)
                Button {
                    isRevealed.toggle()
                } label: {
                    Image(systemName: isRevealed ? "eye.slash" : "eye").foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .help(isRevealed ? "Hide" : "Show")
                .accessibilityLabel(isRevealed ? "Hide secret" : "Show secret")
            }
            .webhookFieldChrome(isFocused: isFocused, isInvalid: problem != nil)

            status
        }
        .animation(.easeOut(duration: 0.15), value: statusKey)
    }

    // MARK: - Menu

    private var useMenu: some View {
        Menu {
            Button("Add to the URL as ?token=", systemImage: "link") { apply(.urlParameter("token")) }
            Divider()
            Button("Send as a Bearer token", systemImage: "key.horizontal") { apply(.bearerToken) }
            Button("Send as an X-API-Key header", systemImage: "key") { apply(.apiKey) }
            Button("Send in a custom header…", systemImage: "list.bullet.rectangle") { apply(.customHeader) }
        } label: {
            Label("Use secret", systemImage: "plus.circle.fill")
                .font(.caption.weight(.semibold))
                .labelStyle(.titleAndIcon)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Put the secret in the URL or in a header")
    }

    private func apply(_ choice: WebhookSecretPlacement.Choice) {
        let edit = WebhookSecretPlacement.apply(choice, url: url, headers: headers)
        url = edit.url
        headers = edit.headers
        if edit.touchesHeaders { headersExpanded = true }
    }

    // MARK: - Status

    private var statusKey: String { "\(isUsed)-\(secret.isEmpty)-\(String(describing: problem))-\(headerUseCount)" }

    @ViewBuilder private var status: some View {
        if let problem {
            Label(WebhookHeaderPolicy.message(for: problem), systemImage: "exclamationmark.triangle.fill")
                .font(.caption).foregroundStyle(.red)
        } else if isUsed && secret.isEmpty {
            Label("Add the secret value that {{secret}} stands for.", systemImage: "exclamationmark.triangle.fill")
                .font(.caption).foregroundStyle(.red)
        } else if isUsed {
            Label(usageDescription, systemImage: "checkmark.circle.fill")
                .font(.caption).foregroundStyle(.green)
        } else if !secret.isEmpty {
            Label("Not used yet. Choose where it goes with Use secret.", systemImage: "info.circle.fill")
                .font(.caption).foregroundStyle(.orange)
        } else {
            Text("Kept in your Keychain and inserted wherever you place it, in the URL or a header.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var usageDescription: String {
        var places: [String] = []
        if urlUsesSecret { places.append("the URL") }
        if headerUseCount > 0 { places.append(headerUseCount == 1 ? "1 header" : "\(headerUseCount) headers") }
        return "Used in " + places.joined(separator: " and ")
    }
}
