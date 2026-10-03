import SwiftUI

/// The replay cursor's date over its time, in monospaced digits so nothing jitters while playing.
/// `isCompact` folds it onto one line for a narrow window.
struct ReplayClockReadout: View {
    let date: Date?
    var isCompact = false

    private var label: String {
        guard let date else { return "No replay time" }
        return "Current replay date and time, \(ReplayStyle.fullText(date))"
    }

    var body: some View {
        Group {
            if let date {
                if isCompact {
                    Text(ReplayStyle.compactText(date))
                        .font(.system(size: 11.5, weight: .medium).monospacedDigit())
                } else {
                    VStack(alignment: .trailing, spacing: 1) {
                        Text(ReplayStyle.dateText(date))
                            .font(.system(size: 10.5, weight: .medium))
                            .foregroundStyle(.secondary)
                        Text(ReplayStyle.timeText(date))
                            .font(.system(size: 13, weight: .semibold).monospacedDigit())
                    }
                }
            } else {
                Text("—").foregroundStyle(.secondary)
            }
        }
        .lineLimit(1)
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
    }
}
