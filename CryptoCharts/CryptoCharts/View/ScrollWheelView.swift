import SwiftUI
import AppKit

/// NSView that captures scroll wheel events and forwards deltaY to a callback.
final class ScrollWheelNSView: NSView {
    var onScroll: ((CGFloat) -> Void)?

    override func scrollWheel(with event: NSEvent) {
        // Only handle if not already at a scroll boundary (avoids conflicts with ScrollView)
        onScroll?(event.scrollingDeltaY)
        super.scrollWheel(with: event)
    }
}

struct ScrollWheelView: NSViewRepresentable {
    let onScroll: (CGFloat) -> Void

    func makeNSView(context: Context) -> ScrollWheelNSView {
        let view = ScrollWheelNSView()
        view.onScroll = onScroll
        return view
    }

    func updateNSView(_ nsView: ScrollWheelNSView, context: Context) {
        nsView.onScroll = onScroll
    }
}

extension View {
    func onScrollWheel(action: @escaping (CGFloat) -> Void) -> some View {
        overlay(ScrollWheelView(onScroll: action).allowsHitTesting(false))
    }
}
