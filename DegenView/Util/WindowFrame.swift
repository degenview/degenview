import Foundation

/// Where a window opens when nothing has been saved for it yet.
///
/// Kept free of AppKit so the arithmetic stands on its own: it takes a screen's
/// visible frame and returns a frame in the same coordinate space.
enum WindowFrame {
    /// Landscape default sized to the screen, capped so a large display doesn't
    /// get a sprawling window and a small one doesn't get an overflowing one.
    static func `default`(in visible: CGRect) -> CGRect {
        let width = min(
            max(visible.width * UI.windowDefaultWidthFraction, UI.windowMinWidth),
            min(UI.windowDefaultMaxWidth, visible.width)
        )
        var height = min(
            max(visible.height * UI.windowDefaultHeightFraction, UI.windowMinHeight),
            min(UI.windowDefaultMaxHeight, visible.height)
        )
        // A short, wide screen must not produce a portrait window.
        height = min(height, width / UI.windowMinAspect)

        // Centered horizontally and sitting a little high — the optical
        // placement `NSWindow.center()` uses.
        let x = visible.midX - width / 2
        let y = visible.maxY - height - (visible.height - height) * 0.35
        return CGRect(x: x.rounded(), y: y.rounded(),
                      width: width.rounded(), height: height.rounded())
    }

    /// Whether a remembered frame still lands on one of `screens` — the display it
    /// was saved on may have been unplugged or rearranged since.
    static func isReachable(_ frame: CGRect, on screens: [CGRect]) -> Bool {
        screens.contains { visible in
            let overlap = visible.intersection(frame)
            return overlap.width >= UI.windowRestoreMinVisibleWidth
                && overlap.height >= UI.windowRestoreMinVisibleHeight
        }
    }
}
