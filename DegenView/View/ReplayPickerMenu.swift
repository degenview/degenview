import SwiftUI

/// A pill menu with a small caption ("Speed", "Resolution"), the current value and a chevron, and a
/// checkmark on the chosen item. Matches the Paper Trading account switcher.
struct ReplayPickerMenu<Value: Hashable & Identifiable>: View {
    let caption: String
    let valueText: String
    let options: [Value]
    let selection: Value?
    let optionTitle: (Value) -> String
    let help: String
    let onSelect: (Value) -> Void
    var isDisabled = false

    var body: some View {
        Menu {
            ForEach(options) { option in
                Button {
                    onSelect(option)
                } label: {
                    if option == selection {
                        Label(optionTitle(option), systemImage: "checkmark")
                    } else {
                        Text(optionTitle(option))
                    }
                }
            }
        } label: {
            HStack(spacing: 5) {
                Text(caption)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.secondary)
                Text(valueText)
                    .font(.system(size: 11.5, weight: .semibold).monospacedDigit())
                Image(systemName: "chevron.down")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(.secondary)
            }
            .fixedSize()
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(
                Color.primary.opacity(0.06),
                in: RoundedRectangle(cornerRadius: ReplayStyle.cornerRadius, style: .continuous))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .disabled(isDisabled)
        .help(help)
        .accessibilityLabel("Replay \(caption.lowercased()), \(valueText)")
    }
}
