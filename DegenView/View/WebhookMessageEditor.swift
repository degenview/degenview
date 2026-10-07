import SwiftUI

/// The webhook body for a price alert: the template, one-tap placeholders, and an example of what
/// will be sent (and whether it goes out as JSON or text).
struct WebhookMessageEditor: View {
    @Binding var text: String
    /// Example values for the preview line. Nil hides it.
    var exampleContext: AlertMessageContext?

    @FocusState private var isFocused: Bool

    private var preview: (text: String, isJSON: Bool)? {
        exampleContext.map { WebhookPickerLogic.examplePreview(template: text, context: $0) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Message").font(.subheadline.weight(.semibold))
                Spacer(minLength: 8)
                if !text.isEmpty {
                    Button("Use default") { text = "" }
                        .buttonStyle(.link)
                        .font(.caption)
                        .transition(.opacity)
                }
            }

            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "text.bubble").foregroundStyle(.secondary).padding(.top, 2)
                TextField(
                    "Message", text: $text, prompt: Text(verbatim: AlertMessageRenderer.defaultPriceAlertTemplate),
                    axis: .vertical
                )
                .textFieldStyle(.plain)
                .font(.body.monospaced())
                .lineLimit(1...4)
                .focused($isFocused)
            }
            .webhookFieldChrome(isFocused: isFocused)

            chips

            if let preview {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text("Example").font(.caption.weight(.medium)).foregroundStyle(.tertiary)
                    Text(preview.text)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .truncationMode(.tail)
                        .textSelection(.enabled)
                    Spacer(minLength: 4)
                    Text(preview.isJSON ? "JSON" : "Text")
                        .font(.caption2.weight(.bold))
                        .foregroundStyle(preview.isJSON ? Color.green : .secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(
                            (preview.isJSON ? Color.green : Color.secondary).opacity(0.14), in: Capsule()
                        )
                        .help(preview.isJSON ? "Sent as application/json" : "Sent as text/plain")
                }
                .accessibilityElement(children: .combine)
            }

            Text("Sent as the request body. Leave empty for the default.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .animation(.easeOut(duration: 0.15), value: text.isEmpty)
    }

    /// Tap to add a placeholder to the message.
    private var chips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(AlertMessageRenderer.supportedPlaceholders, id: \.self) { name in
                    PlaceholderChip(name: name) {
                        text = WebhookPickerLogic.appending("{{\(name)}}", to: text)
                    }
                }
            }
            .padding(.trailing, 16)
        }
        .mask(
            HStack(spacing: 0) {
                Rectangle()
                LinearGradient(colors: [.black, .clear], startPoint: .leading, endPoint: .trailing)
                    .frame(width: 18)
            })
    }
}

private struct PlaceholderChip: View {
    let name: String
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            Text(verbatim: "{{\(name)}}")
                .font(.caption.monospaced().weight(.medium))
                .foregroundStyle(isHovered ? Color.accentColor : .secondary)
                .padding(.horizontal, 9)
                .padding(.vertical, 4)
                .background(
                    (isHovered ? Color.accentColor : Color.primary).opacity(isHovered ? 0.14 : 0.06), in: Capsule())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .help("Add {{\(name)}} to the message")
        .accessibilityLabel("Add \(name)")
    }
}
