import AppKit

/// Drag a chart's price axis to scale it: up for a narrower price slice (taller candles), down for
/// a wider one. Double-click restores auto-fit.
///
/// Runs off mouse events seen by a monitor rather than an `NSView`'s own handlers, because
/// SwiftUI's hosting view claims these events for a card's `.onDrag` reordering before AppKit
/// offers them to any child view. Swallowing a gesture that lands on a gutter is what keeps that
/// reorder drag from starting.
///
/// A chart tab feeds it from its own mouse monitor, which must offer the gutter first refusal
/// (`handle(_:)`); a host with nothing else to arbitrate calls `install()` instead.
@MainActor
final class PriceAxisDragMonitor {
    weak var window: NSWindow? {
        didSet { if window !== oldValue { target = nil } }
    }

    /// True while something modal is up and the gutters shouldn't react.
    var isSuspended: () -> Bool = { false }
    /// A drag or reset finished: the one moment to persist, rather than on every mouse move.
    var onChangeEnded: () -> Void = {}

    /// Y-axis gutters, each mapped to the chart it scales. Weak on both sides, so a removed card
    /// drops out on its own.
    private let regions = NSMapTable<NSView, ChartViewModel>.weakToWeakObjects()
    /// The chart whose axis is being dragged right now, and where the drag started.
    private var target: ChartViewModel?
    private var origin: NSPoint = .zero
    private var didDrag = false
    private var monitor: Any?

    func register(_ view: NSView, for chart: ChartViewModel) {
        regions.setObject(chart, forKey: view)
    }

    /// Handles mouse events on its own, for hosts where nothing else needs them.
    func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseDragged, .leftMouseUp]) {
            [weak self] event in
            self?.handle(event)
        }
    }

    /// Returns nil when the event was consumed by the axis, else the event to pass on.
    func handle(_ event: NSEvent) -> NSEvent? {
        guard !isSuspended(), let own = window, event.window === own else { return event }

        switch event.type {
        case .leftMouseDown:
            guard let chart = chart(at: event) else { return event }
            guard event.clickCount < 2 else {
                target = nil
                chart.resetYZoom()
                onChangeEnded()
                return nil
            }
            target = chart
            // Window coordinates are y-up, so the delta is already "positive = up".
            origin = event.locationInWindow
            didDrag = false
            chart.beginYZoomDrag()
            return nil

        case .leftMouseDragged:
            guard let target else { return event }
            didDrag = true
            target.updateYZoom(dragOffset: event.locationInWindow.y - origin.y)
            return nil

        case .leftMouseUp:
            guard target != nil else { return event }
            target = nil
            if didDrag { onChangeEnded() }
            didDrag = false
            return nil

        default:
            return event
        }
    }

    /// The chart whose price-axis gutter sits under this event, if any.
    private func chart(at event: NSEvent) -> ChartViewModel? {
        guard let window = event.window, let content = window.contentView else { return nil }
        let point = event.locationInWindow
        guard window.contentLayoutRect.contains(content.convert(point, from: nil)) else { return nil }
        guard let views = regions.keyEnumerator().allObjects as? [NSView] else { return nil }
        for view in views {
            guard view.window === window, !view.isHiddenOrHasHiddenAncestor else { continue }
            if view.bounds.contains(view.convert(point, from: nil)) {
                return regions.object(forKey: view)
            }
        }
        return nil
    }

    deinit {
        if let monitor { NSEvent.removeMonitor(monitor) }
    }
}
