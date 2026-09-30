import SwiftUI

extension Color {
    /// A Pine color: `0xRRGGBBAA`.
    init(pineRGBA value: UInt32) {
        self.init(
            .sRGB,
            red: Double((value >> 24) & 255) / 255,
            green: Double((value >> 16) & 255) / 255,
            blue: Double((value >> 8) & 255) / 255,
            opacity: Double(value & 255) / 255)
    }

    /// This color as `0xRRGGBBAA` in sRGB, or nil when it has no sRGB representation.
    var pineRGBA: UInt32? {
        guard let color = NSColor(self).usingColorSpace(.sRGB) else { return nil }
        return UInt32((color.redComponent * 255).rounded()) << 24
            | UInt32((color.greenComponent * 255).rounded()) << 16
            | UInt32((color.blueComponent * 255).rounded()) << 8
            | UInt32((color.alphaComponent * 255).rounded())
    }
}
