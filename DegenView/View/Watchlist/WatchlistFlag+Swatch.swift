import AppKit
import SwiftUI

extension WatchlistFlag {
    /// The flag as a small full-colour image, for menu items. Not a template, so AppKit keeps its colour.
    @MainActor
    var swatch: NSImage {
        if let cached = Self.swatches[self] { return cached }
        let renderer = ImageRenderer(content: WatchlistFlagMark(flag: self, size: CGSize(width: 12, height: 10)))
        renderer.scale = 3
        let image = renderer.nsImage ?? NSImage()
        image.isTemplate = false
        Self.swatches[self] = image
        return image
    }

    @MainActor private static var swatches: [WatchlistFlag: NSImage] = [:]
}
