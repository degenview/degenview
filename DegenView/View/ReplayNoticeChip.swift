import SwiftUI

/// A non-blocking warning from preparing the replay (a chart fell back to complete bars), with a
/// dismiss button.
struct ReplayNoticeChip: View {
    let message: String
    let onDismiss: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 11))
                .foregroundStyle(ReplayStyle.accent)
            Text(message)
                .font(.system(size: 11.5))
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(action: onDismiss) {
                Image(systemName: "xmark").font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.secondary)
                    .frame(width: 18, height: 18)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Dismiss")
            .accessibilityLabel("Dismiss replay notice")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(
            ReplayStyle.accent.opacity(0.10),
            in: RoundedRectangle(cornerRadius: ReplayStyle.cornerRadius, style: .continuous)
        )
        .accessibilityElement(children: .combine)
    }
}
