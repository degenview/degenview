import SwiftUI

/// The "REPLAY" marker plus what the replay is doing right now. Always says it in words as well as
/// colour, and the playing dot only pulses when Reduce Motion is off.
struct ReplayStatusChip: View {
    let status: ReplayStatus
    var isPreparing = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isPulsing = false

    private var tint: Color { ReplayStyle.tint(for: status, isPreparing: isPreparing) }
    private var title: String { ReplayStyle.title(for: status, isPreparing: isPreparing) }

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "clock.arrow.circlepath").font(.system(size: 10, weight: .bold))
            Text("REPLAY").font(.system(size: 10.5, weight: .bold)).tracking(0.4)
            Rectangle().fill(tint.opacity(0.35)).frame(width: 1, height: 10)
            indicator
            Text(title).font(.system(size: 11, weight: .medium)).lineLimit(1)
        }
        .foregroundStyle(ReplayStyle.accent)
        .padding(.horizontal, 9)
        .padding(.vertical, 4)
        .background(ReplayStyle.accent.opacity(0.14), in: Capsule())
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Historical replay, \(title)")
    }

    @ViewBuilder
    private var indicator: some View {
        if isPreparing {
            ProgressView().controlSize(.mini).scaleEffect(0.8).frame(width: 8, height: 8)
        } else {
            Circle()
                .fill(tint)
                .frame(width: 6, height: 6)
                .opacity(status == .playing && isPulsing ? 0.35 : 1)
                .animation(
                    status == .playing && !reduceMotion
                        ? .easeInOut(duration: 0.8).repeatForever(autoreverses: true) : .default,
                    value: isPulsing
                )
                .onAppear { isPulsing = true }
        }
    }
}
