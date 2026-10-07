import SwiftUI

/// Create or edit a webhook endpoint: a name, where to post, an optional secret and headers, and
/// whether it is on.
struct WebhookEndpointEditorSheet: View {
    private enum Field: Hashable { case name, url, description }

    /// The tallest the form grows before it scrolls.
    private static let maximumFormHeight: CGFloat = 440

    /// Nil creates a new endpoint.
    let existing: WebhookEndpoint?
    @ObservedObject var store: WebhookEndpointStore

    @Environment(\.dismiss) private var dismiss
    @FocusState private var focus: Field?
    @State private var name: String
    @State private var description: String
    @State private var url: String
    @State private var method: WebhookHTTPMethod
    @State private var secret: String
    @State private var headers: [WebhookHeader]
    @State private var headersExpanded: Bool
    @State private var isEnabled: Bool
    @State private var failure: String?

    init(existing: WebhookEndpoint?, store: WebhookEndpointStore) {
        self.existing = existing
        self.store = store
        _name = State(initialValue: existing?.name ?? "")
        _description = State(initialValue: existing?.description ?? "")
        _url = State(initialValue: existing?.url ?? "")
        _method = State(initialValue: existing?.method ?? .post)
        _secret = State(initialValue: existing.flatMap { store.secret(for: $0.id) } ?? "")
        _headers = State(initialValue: existing?.headers ?? [])
        _headersExpanded = State(initialValue: !(existing?.headers.isEmpty ?? true))
        _isEnabled = State(initialValue: existing?.isEnabled ?? true)
    }

    // MARK: - Validation

    private var trimmedURL: String { url.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var filledHeaders: [WebhookHeader] { headers.filter { !$0.isBlank } }

    private var urlProblem: String? {
        guard !trimmedURL.isEmpty else { return nil }
        if let problem = WebhookRequestResolver.validateTemplates(url: trimmedURL, headers: []) {
            return WebhookURLPolicy.message(for: problem)
        }
        return nil
    }

    private var urlIsValid: Bool { !trimmedURL.isEmpty && urlProblem == nil }

    /// A half-typed address is not an error yet: the message waits until the field loses focus.
    private var visibleURLProblem: String? { focus == .url ? nil : urlProblem }

    private var usesSecret: Bool { WebhookRequestResolver.references(url: trimmedURL, headers: filledHeaders) }

    private var secretIsValid: Bool {
        (secret.isEmpty || WebhookHeaderPolicy.validate(secret: secret) == nil) && (!usesSecret || !secret.isEmpty)
    }

    private var canSave: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && urlIsValid
            && WebhookHeaderPolicy.isValid(filledHeaders) && secretIsValid
    }

    /// A header that looks like a credential, typed in plain while no secret is saved.
    private var plainCredential: WebhookHeader? {
        secret.isEmpty ? WebhookSecretPlacement.plainCredentialHeader(in: filledHeaders) : nil
    }

    // MARK: - Body

    var body: some View {
        FormScrollContainer(maxHeight: Self.maximumFormHeight) {
            form
        } footer: {
            footer
        }
        .frame(width: 480)
        .onAppear { focus = existing == nil ? .name : nil }
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 18) {
            SheetHeader(
                systemImage: "arrow.up.forward.app.fill", title: existing == nil ? "Add Webhook" : "Edit Webhook",
                subtitle: "Alerts post their message here over HTTP.")

            VStack(alignment: .leading, spacing: 8) {
                sectionTitle("Name")
                field(focused: focus == .name) {
                    Image(systemName: "tag").foregroundStyle(.secondary)
                    TextField("Trading Bot", text: $name)
                        .textFieldStyle(.plain)
                        .focused($focus, equals: .name)
                }
            }

            urlSection

            WebhookSecretSection(
                secret: $secret, url: $url, headers: $headers, headersExpanded: $headersExpanded)

            if let credential = plainCredential {
                credentialNudge(credential)
            }

            WebhookHeadersEditor(headers: $headers, isExpanded: $headersExpanded)

            VStack(alignment: .leading, spacing: 8) {
                sectionTitle("Description")
                field(focused: focus == .description) {
                    Image(systemName: "text.alignleft").foregroundStyle(.secondary)
                    TextField("Optional", text: $description)
                        .textFieldStyle(.plain)
                        .focused($focus, equals: .description)
                }
            }

            enabledCard

            if let failure {
                SettingsStatusMessage(text: failure, isError: true)
            }
        }
    }

    private var urlSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionTitle("Webhook URL")
            field(focused: focus == .url, isInvalid: visibleURLProblem != nil) {
                methodMenu
                Divider().frame(height: 16)
                TextField("Webhook URL", text: $url, prompt: Text(verbatim: "https://example.com/webhook"))
                    .textFieldStyle(.plain)
                    .font(.body.monospaced())
                    .autocorrectionDisabled()
                    .focused($focus, equals: .url)
                if urlIsValid {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                        .transition(.scale.combined(with: .opacity))
                        .accessibilityLabel("Valid address")
                }
            }
            if let problem = visibleURLProblem {
                Label(problem, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(.red)
                    .transition(.opacity)
            } else if urlIsValid, WebhookSecretPlaceholder.contains(trimmedURL) {
                preview
            }
        }
        .animation(.easeOut(duration: 0.15), value: urlIsValid)
        .animation(.easeOut(duration: 0.15), value: visibleURLProblem)
    }

    /// What will actually be requested, with the secret hidden.
    private var preview: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: "arrow.turn.down.right").font(.caption).foregroundStyle(.tertiary)
            Text(WebhookRequestResolver.maskedURL(trimmedURL))
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .truncationMode(.middle)
                .textSelection(.enabled)
        }
        .transition(.opacity)
        .accessibilityLabel("Sends to \(WebhookRequestResolver.maskedURL(trimmedURL))")
    }

    // MARK: - Pieces

    /// The HTTP method as a compact dropdown at the front of the URL field.
    private var methodMenu: some View {
        Menu {
            Picker("Method", selection: $method) {
                ForEach(WebhookHTTPMethod.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.inline)
            .labelsHidden()
        } label: {
            HStack(spacing: 5) {
                Text(method.rawValue).font(.system(size: 12, weight: .bold, design: .monospaced))
                Image(systemName: "chevron.up.chevron.down").font(.system(size: 8, weight: .bold))
            }
            .foregroundStyle(Color.accentColor)
            .contentShape(Rectangle())
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("HTTP method")
        .accessibilityLabel("HTTP method, \(method.rawValue)")
    }

    private func credentialNudge(_ header: WebhookHeader) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "key.fill").foregroundStyle(.orange).padding(.top, 2)
            VStack(alignment: .leading, spacing: 6) {
                Text("“\(header.name)” looks like a credential")
                    .font(.subheadline.weight(.semibold))
                Text("Save it as the secret so it is kept in your Keychain instead of in plain text.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Move to Secret") { moveToSecret(header) }
                    .controlSize(.small)
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .background(Color.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(Color.orange.opacity(0.35)))
        .transition(.opacity)
    }

    private func moveToSecret(_ header: WebhookHeader) {
        let moved = WebhookSecretPlacement.movingToSecret(header)
        secret = moved.secret
        if let index = headers.firstIndex(where: { $0.id == header.id }) { headers[index].value = moved.value }
    }

    private var enabledCard: some View {
        HStack(spacing: 12) {
            Image(systemName: isEnabled ? "bolt.fill" : "bolt.slash.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(isEnabled ? Color.accentColor : .secondary)
                .frame(width: 30, height: 30)
                .background(
                    (isEnabled ? Color.accentColor : Color.primary).opacity(0.12),
                    in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                )
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("Enabled").font(.subheadline.weight(.semibold))
                Text(isEnabled ? "Alerts that use this webhook will post to it." : "Paused: alerts skip this webhook.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Toggle("Enabled", isOn: $isEnabled)
                .labelsHidden()
                .toggleStyle(.switch)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 11)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(.separator.opacity(0.5)))
        .animation(.easeOut(duration: 0.15), value: isEnabled)
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Spacer()
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .controlSize(.large)
            Button(existing == nil ? "Add Webhook" : "Save Changes", action: save)
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .controlSize(.large)
                .disabled(!canSave)
        }
    }

    private func sectionTitle(_ text: String) -> some View {
        Text(text).font(.subheadline.weight(.semibold))
    }

    private func field<Content: View>(
        focused: Bool, isInvalid: Bool = false, @ViewBuilder _ content: () -> Content
    ) -> some View {
        HStack(spacing: 8) { content() }
            .webhookFieldChrome(isFocused: focused, isInvalid: isInvalid)
    }

    // MARK: - Actions

    private func save() {
        do {
            if let existing {
                try store.update(
                    id: existing.id, name: name, description: description, url: url, method: method,
                    headers: filledHeaders, secret: secret, isEnabled: isEnabled)
            } else {
                try store.add(
                    name: name, description: description, url: url, method: method, headers: filledHeaders,
                    secret: secret, isEnabled: isEnabled)
            }
            dismiss()
        } catch {
            failure = error.localizedDescription
        }
    }
}
