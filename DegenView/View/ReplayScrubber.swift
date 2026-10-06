import SwiftUI

/// The replay timeline: a track filled up to the cursor, a tick where the replay started, and a thumb
/// to drag. Click anywhere to jump. Seeking pauses, like the transport's own step buttons.
struct ReplayScrubber: View {
    @ObservedObject var engine: ReplayEngine
    var isDisabled = false
    @State private var isHovering = false
    @State private var isDragging = false

    private let trackHeight: CGFloat = 4

    var body: some View {
        GeometryReader { geometry in
            let width = max(geometry.size.width, 1)
            let progress = engine.progress
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(Color.primary.opacity(0.12))
                    .frame(height: trackHeight)
                Capsule()
                    .fill(ReplayStyle.accent)
                    .frame(width: max(progress * width, 0), height: trackHeight)
                Rectangle()
                    .fill(Color.primary.opacity(0.45))
                    .frame(width: 2, height: 10)
                    .offset(x: engine.startFraction * width - 1)
                Circle()
                    .fill(.white)
                    .overlay(Circle().stroke(ReplayStyle.accent, lineWidth: 2))
                    .frame(width: isHovering || isDragging ? 13 : 11, height: isHovering || isDragging ? 13 : 11)
                    .shadow(color: .black.opacity(0.25), radius: 1.5, y: 0.5)
                    .offset(x: progress * width - (isHovering || isDragging ? 6.5 : 5.5))
            }
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        isDragging = true
                        engine.seek(toFraction: value.location.x / width)
                    }
                    .onEnded { _ in isDragging = false }
            )
            .onHover { isHovering = $0 }
            .animation(.easeOut(duration: 0.12), value: isHovering)
        }
        .frame(height: 18)
        .opacity(isDisabled ? 0.4 : 1)
        .allowsHitTesting(!isDisabled)
        .help("Step \(engine.barNumber) of \(engine.barCount)")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Replay position")
        .accessibilityValue("Step \(engine.barNumber) of \(engine.barCount)")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: engine.stepForward()
            case .decrement: engine.stepBackward()
            @unknown default: break
            }
        }
    }
}
