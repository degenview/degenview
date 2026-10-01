import AppKit

/// Turns scroll-wheel events over one view into zoom steps.
///
/// A local `NSEvent` monitor sees every scroll in the app, so it answers only for events in its
/// own window and inside the registered region — otherwise a scroll aimed at a chart tab would
/// zoom the preview too.
@MainActor
final class ScrollZoomMonitor {
    /// Receives +1 to zoom in (scroll up) and -1 to zoom out.
    var onScroll: (Int) -> Void = { _ in }

    weak var region: NSView?
    weak var window: NSWindow? {
        didSet { if window != nil { install() } }
    }

    private var monitor: Any?

    private func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
            guard let self, let steps = steps(for: event) else { return event }
            onScroll(steps)
            return event
        }
    }

    /// `bounds`, not `visibleRect`: SwiftUI's superviews don't clip their subviews, so a
    /// `visibleRect` would cover the whole window and match every scroll.
    private func steps(for event: NSEvent) -> Int? {
        guard let window, event.window === window, let region, region.window === window,
            !region.isHiddenOrHasHiddenAncestor, event.scrollingDeltaY != 0,
            region.bounds.contains(region.convert(event.locationInWindow, from: nil))
        else { return nil }
        return event.scrollingDeltaY > 0 ? 1 : -1
    }

    deinit {
        if let monitor { NSEvent.removeMonitor(monitor) }
    }
}
