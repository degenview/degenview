import SwiftUI

struct ReplayControlBar: View {
    @ObservedObject var engine: ReplayEngine
    let onChangeStart: () -> Void
    let onReturnToLive: () -> Void
    let onClose: () -> Void
    var availableIntervals: [ReplayInterval] = [.automatic, .chartBar]
    let onIntervalChanged: (ReplayInterval) -> Void
    var isPreparing = false

    var body: some View {
        HStack(spacing: 6) {
            Button(action: onChangeStart) { Image(systemName: "cursorarrow.click.2") }
                .accessibilityLabel("Select replay starting point")
                .help("Change starting point")

            Button {
                engine.stepForward()
            } label: {
                Image(systemName: "forward.frame.fill")
            }
            .keyboardShortcut(.rightArrow, modifiers: .shift)
            .disabled(!engine.canAdvance)
            .accessibilityLabel("Advance replay one step")
            .help("Step Forward (Shift–Right Arrow)")

            Button {
                engine.togglePlayback()
            } label: {
                Image(systemName: engine.status == .playing ? "pause.fill" : "play.fill")
            }
            .keyboardShortcut(.downArrow, modifiers: .shift)
            .disabled(!(engine.status == .paused || engine.status == .playing))
            .accessibilityLabel(engine.status == .playing ? "Pause historical replay" : "Play historical replay")
            .help("Play/Pause (Shift–Down Arrow)")

            Menu(engine.session?.playbackSpeed.rawValue ?? ReplaySpeed.normal.rawValue) {
                ForEach(ReplaySpeed.allCases) { speed in
                    Button {
                        engine.setSpeed(speed)
                    } label: {
                        if engine.session?.playbackSpeed == speed {
                            Label(speed.rawValue, systemImage: "checkmark")
                        } else {
                            Text(speed.rawValue)
                        }
                    }
                }
            }
            .accessibilityLabel("Replay playback speed")
            .help("Playback speed")

            Menu(engine.session?.replayInterval.rawValue ?? ReplayInterval.automatic.rawValue) {
                ForEach(availableIntervals) { interval in
                    Button {
                        onIntervalChanged(interval)
                    } label: {
                        if engine.session?.replayInterval == interval {
                            Label(interval.rawValue, systemImage: "checkmark")
                        } else {
                            Text(interval.rawValue)
                        }
                    }
                }
            }
            .accessibilityLabel("Replay interval")
            .help("Available historical replay intervals")

            if isPreparing {
                ProgressView()
                    .controlSize(.small)
                    .accessibilityLabel("Loading granular replay history")
            }

            Divider().frame(height: 16)

            Label {
                Text(timestampText)
                    .font(.caption.monospacedDigit())
            } icon: {
                Image(systemName: "clock.arrow.circlepath")
                    .foregroundStyle(.orange)
            }
            .accessibilityLabel("Current replay date and time, \(timestampText)")

            if engine.status == .completed {
                Text("End reached")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.orange)
                    .accessibilityLabel("Historical replay reached the end")
                Button("Restart") { engine.restart() }
                    .accessibilityLabel("Restart historical replay")
            }

            Spacer(minLength: 8)

            Button(action: onReturnToLive) { Label("Latest", systemImage: "dot.radiowaves.left.and.right") }
                .accessibilityLabel("Return to live market")
                .help("Jump to latest and leave replay")

            Button(action: onClose) { Image(systemName: "xmark") }
                .accessibilityLabel("Close historical replay")
                .help("Close Replay")
        }
        .buttonStyle(.borderless)
        .controlSize(.small)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.ultraThinMaterial)
        .overlay(alignment: .bottom) { Divider() }
    }

    private var timestampText: String {
        guard let date = engine.currentTimestamp else {
            return engine.status == .selectingStart ? "Select a historical bar" : "Replay"
        }
        return date.formatted(date: .abbreviated, time: .standard)
    }
}
