import SwiftUI

/// A coloured dot and a word: "Connected", "Not configured", "Needs approval".
struct SettingsStatusBadge: View {
    enum Tone {
        case good, warning, bad, neutral

        var color: Color {
            switch self {
            case .good: .green
            case .warning: .orange
            case .bad: .red
            case .neutral: .secondary
            }
        }
    }

    let text: String
    let tone: Tone

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(tone.color).frame(width: 7, height: 7)
            Text(text).font(.caption.weight(.semibold)).foregroundStyle(tone.color)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 4)
        .background(tone.color.opacity(0.12), in: Capsule())
        .accessibilityElement(children: .combine)
    }
}

/// The result line beside a page's Save button: green after success, red after a failure.
struct SettingsStatusMessage: View {
    let text: String
    var isError = false

    var body: some View {
        Label(text, systemImage: isError ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
            .font(.caption)
            .foregroundStyle(isError ? Color.red : Color.green)
            .lineLimit(2)
    }
}
