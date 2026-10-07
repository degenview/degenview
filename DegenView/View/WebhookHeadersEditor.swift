import SwiftUI

/// Custom HTTP headers for a webhook. Collapsed by default: most webhooks need none, and the ones that
/// do usually need one. Each row is a name and a value; a value may use `{{secret}}`.
struct WebhookHeadersEditor: View {
    @Binding var headers: [WebhookHeader]
    @Binding var isExpanded: Bool

    private enum Focus: Hashable {
        case name(UUID)
        case value(UUID)
    }

    @FocusState private var focus: Focus?

    private var errors: [UUID: WebhookHeaderError] { WebhookHeaderPolicy.errors(in: headers.filter { !$0.isBlank }) }
    private var canAdd: Bool { headers.count < WebhookHeaderPolicy.maximumHeaders }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            toggle
            if isExpanded {
                VStack(alignment: .leading, spacing: 8) {
                    if headers.isEmpty {
                        Text("No custom headers yet.").font(.caption).foregroundStyle(.secondary)
                    }
                    ForEach(headers) { header in
                        row(header.id)
                    }
                    Button(action: addHeader) {
                        Label("Add header", systemImage: "plus")
                            .font(.caption.weight(.semibold))
                    }
                    .buttonStyle(.borderless)
                    .disabled(!canAdd)
                    .help(canAdd ? "Add a header" : "At most \(WebhookHeaderPolicy.maximumHeaders) headers")
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .animation(.easeOut(duration: 0.18), value: isExpanded)
        .animation(.easeOut(duration: 0.18), value: headers.map(\.id))
        .onChange(of: headers.map(\.id)) { old, new in
            // A row added elsewhere (the Use secret menu) gets the cursor in its blank name.
            guard new.count > old.count, let added = headers.last(where: { !old.contains($0.id) }),
                added.name.isEmpty
            else { return }
            focus = .name(added.id)
        }
    }

    // MARK: - Pieces

    private var toggle: some View {
        Button {
            isExpanded.toggle()
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    .frame(width: 12)
                Text("Headers").font(.subheadline.weight(.semibold))
                if !headers.isEmpty {
                    Text("\(headers.count)")
                        .font(.caption2.weight(.bold).monospacedDigit())
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 1)
                        .background(Color.primary.opacity(0.08), in: Capsule())
                }
                if !isExpanded && !errors.isEmpty {
                    Image(systemName: "exclamationmark.triangle.fill").font(.caption).foregroundStyle(.red)
                }
                Text("Optional").font(.caption).foregroundStyle(.tertiary)
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Headers, \(headers.count) set")
        .accessibilityHint(isExpanded ? "Collapse" : "Expand")
    }

    /// A binding to one field of the header with `id`, found by id on every access. An index binding
    /// (`ForEach($headers)`) is read once more after its row deletes itself and then points past the
    /// end of the array.
    private func binding(_ id: UUID, _ keyPath: WritableKeyPath<WebhookHeader, String>) -> Binding<String> {
        Binding(
            get: { headers.first { $0.id == id }?[keyPath: keyPath] ?? "" },
            set: { value in
                if let index = headers.firstIndex(where: { $0.id == id }) { headers[index][keyPath: keyPath] = value }
            })
    }

    private func row(_ id: UUID) -> some View {
        let error = errors[id]
        return VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                TextField("Header-Name", text: binding(id, \.name))
                    .textFieldStyle(.plain)
                    .font(.body.monospaced())
                    .autocorrectionDisabled()
                    .focused($focus, equals: .name(id))
                    .webhookFieldChrome(isFocused: focus == .name(id), isInvalid: error != nil && isNameError(error))
                    .frame(width: 150)
                HStack(spacing: 6) {
                    TextField("Value", text: binding(id, \.value))
                        .textFieldStyle(.plain)
                        .font(.body.monospaced())
                        .autocorrectionDisabled()
                        .focused($focus, equals: .value(id))
                    Button {
                        if let index = headers.firstIndex(where: { $0.id == id }) {
                            headers[index].value += WebhookSecretPlaceholder.token
                        }
                    } label: {
                        Image(systemName: "key.horizontal").foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help("Insert the secret")
                    .accessibilityLabel("Insert the secret into this value")
                }
                .webhookFieldChrome(isFocused: focus == .value(id), isInvalid: error != nil && !isNameError(error))
                SettingsIconButton(systemImage: "trash", label: "Remove header", isDestructive: true) {
                    headers.removeAll { $0.id == id }
                }
            }
            if let error {
                Label(WebhookHeaderPolicy.message(for: error), systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }

    private func isNameError(_ error: WebhookHeaderError?) -> Bool {
        switch error {
        case .emptyName, .invalidName, .nameTooLong, .reserved, .duplicate: true
        case .invalidValue, .valueTooLong, nil: false
        }
    }

    private func addHeader() {
        let header = WebhookHeader()
        headers.append(header)
        focus = .name(header.id)
    }
}
