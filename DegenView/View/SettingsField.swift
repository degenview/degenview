import SwiftUI

/// A labelled text field in the app's field style. A secret field hides its text and offers an
/// eye button to reveal it, so a pasted key can be checked before saving.
struct SettingsField: View {
    let title: String
    var hint: String?
    var prompt = ""
    @Binding var text: String
    var isSecret = false
    @State private var isRevealed = false
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.subheadline.weight(.medium))
            HStack(spacing: 8) {
                field
                    .textFieldStyle(.plain)
                    .focused($focused)
                    .autocorrectionDisabled()
                if isSecret {
                    Button {
                        isRevealed.toggle()
                    } label: {
                        Image(systemName: isRevealed ? "eye.slash" : "eye").foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help(isRevealed ? "Hide" : "Show")
                    .accessibilityLabel(isRevealed ? "Hide \(title)" : "Show \(title)")
                }
            }
            .padding(.horizontal, 10)
            .frame(minHeight: UI.searchFieldHeight)
            .background(.background.opacity(0.7), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(focused ? Color.accentColor.opacity(0.7) : Color.primary.opacity(0.15))
            )
            if let hint {
                Text(hint).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder private var field: some View {
        if isSecret && !isRevealed {
            SecureField(prompt, text: $text)
        } else {
            TextField(prompt, text: $text)
        }
    }
}
