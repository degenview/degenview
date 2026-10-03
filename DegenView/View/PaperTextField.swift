import SwiftUI

/// A text input with a unit on its right edge ("BTC", "USD", "×"), drawn like the cards around it.
/// A red outline marks a value the form will not accept.
struct PaperTextField: View {
    let placeholder: String
    @Binding var text: String
    var suffix: String?
    var isError = false
    var isMonospaced = true
    /// A small button inside the field's trailing edge ("Last"), for filling it from the market.
    var accessoryTitle: String?
    var accessoryAction: (() -> Void)?
    @FocusState private var isFocused: Bool

    var body: some View {
        HStack(spacing: 8) {
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .font(isMonospaced ? .body.monospacedDigit() : .body)
                .focused($isFocused)
            if let accessoryTitle, let accessoryAction {
                Button(accessoryTitle, action: accessoryAction)
                    .buttonStyle(.plain)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Color.accentColor)
            }
            if let suffix {
                Text(suffix).font(.callout).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .strokeBorder(borderColor, lineWidth: isFocused || isError ? 1.5 : 1)
        }
        .contentShape(Rectangle())
        .onTapGesture { isFocused = true }
    }

    private var borderColor: Color {
        if isError { return .red.opacity(0.8) }
        return isFocused ? Color.accentColor.opacity(0.8) : Color(nsColor: .separatorColor).opacity(0.6)
    }
}
