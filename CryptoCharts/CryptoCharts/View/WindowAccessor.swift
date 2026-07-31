import SwiftUI
import AppKit

/// Zero-size bridge that hands the hosting `NSWindow` to SwiftUI.
///
/// Three things need it: joining the window to a tab group, scoping the
/// scroll-zoom monitor to its own window, and watching occlusion so hidden tabs
/// stop polling.
struct WindowAccessor: NSViewRepresentable {
    let onResolve: (NSWindow) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = CaptureView()
        view.onResolve = onResolve
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        (nsView as? CaptureView)?.onResolve = onResolve
    }

    private final class CaptureView: NSView {
        var onResolve: ((NSWindow) -> Void)?
        private weak var lastWindow: NSWindow?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window, window !== lastWindow else { return }
            lastWindow = window
            // The window isn't ordered in yet on the first pass; tab-group joins
            // need it on screen, so let the run loop settle first.
            DispatchQueue.main.async { [weak self, weak window] in
                guard let self, let window else { return }
                self.onResolve?(window)
            }
        }

        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}
