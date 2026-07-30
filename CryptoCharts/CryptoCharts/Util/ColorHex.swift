import SwiftUI

extension Color {
    /// Hex string (e.g. "#FF0000" or "FF0000") → Color. Falls back to gray on invalid input.
    init(hex: String) {
        let trimmed = hex.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "#", with: "")
        var hexValue: UInt64 = 0
        guard Scanner(string: trimmed).scanHexInt64(&hexValue) else {
            self = .gray
            return
        }
        let r, g, b, a: Double
        switch trimmed.count {
        case 6:
            r = Double((hexValue & 0xFF0000) >> 16) / 255.0
            g = Double((hexValue & 0x00FF00) >> 8) / 255.0
            b = Double(hexValue & 0x0000FF) / 255.0
            a = 1.0
        case 8:
            a = Double((hexValue & 0xFF000000) >> 24) / 255.0
            r = Double((hexValue & 0x00FF0000) >> 16) / 255.0
            g = Double((hexValue & 0x0000FF00) >> 8) / 255.0
            b = Double(hexValue & 0x000000FF) / 255.0
        default:
            self = .gray
            return
        }
        self.init(.sRGB, red: r, green: g, blue: b, opacity: a)
    }

    /// Returns hex string like "#FF0000". Returns nil if color can't be resolved.
    var hexString: String? {
        guard let cgColor = NSColor(self).usingColorSpace(.sRGB)?.cgColor,
              let components = cgColor.components,
              components.count >= 3
        else { return nil }

        let r = UInt8((components[0] * 255).rounded().clamped(to: 0...255))
        let g = UInt8((components[1] * 255).rounded().clamped(to: 0...255))
        let b = UInt8((components[2] * 255).rounded().clamped(to: 0...255))
        return String(format: "#%02X%02X%02X", r, g, b)
    }
}
