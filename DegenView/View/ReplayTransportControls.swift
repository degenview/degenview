import SwiftUI

/// Restart, step back, play/pause and step forward. Play/pause is the one filled control; at the end
/// of the data it becomes "replay again".
struct ReplayTransportControls: View {
    @ObservedObject var engine: ReplayEngine
    var isPreparing = false

    private var isPlaying: Bool { engine.status == .playing }
    private var isEnded: Bool { engine.status == .completed }
    private var canTogglePlayback: Bool {
        !isPreparing && (engine.status == .paused || engine.status == .playing || isEnded)
    }

    var body: some View {
        HStack(spacing: 2) {
            ReplayIconButton(systemImage: "backward.end.fill", label: "Back to start") {
                engine.restart()
            }
            .disabled(isPreparing || engine.session == nil)

            ReplayIconButton(
                systemImage: "backward.frame.fill", label: "Step Back (Shift–Left Arrow)",
                shortcut: KeyboardShortcut(.leftArrow, modifiers: .shift)
            ) {
                engine.stepBackward()
            }
            .disabled(isPreparing || !engine.canStepBackward)

            playButton

            ReplayIconButton(
                systemImage: "forward.frame.fill", label: "Step Forward (Shift–Right Arrow)",
                shortcut: KeyboardShortcut(.rightArrow, modifiers: .shift)
            ) {
                engine.stepForward()
            }
            .disabled(isPreparing || !engine.canAdvance)
        }
    }

    private var playButton: some View {
        let action = isEnded ? "Replay Again" : isPlaying ? "Pause" : "Play"
        let label = "\(action) (Shift–Down Arrow)"
        return Button {
            if isEnded { engine.restart() }
            engine.togglePlayback()
        } label: {
            Image(systemName: isEnded ? "arrow.counterclockwise" : isPlaying ? "pause.fill" : "play.fill")
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(.white)
                .frame(width: 28, height: 28)
                .background(Circle().fill(ReplayStyle.accent))
                .contentTransition(.symbolEffect(.replace))
                .opacity(canTogglePlayback ? 1 : 0.4)
        }
        .buttonStyle(.plain)
        .keyboardShortcut(.downArrow, modifiers: .shift)
        .disabled(!canTogglePlayback)
        .padding(.horizontal, 2)
        .help(label)
        .accessibilityLabel(label)
    }
}
