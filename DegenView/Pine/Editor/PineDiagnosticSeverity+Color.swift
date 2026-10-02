import AppKit
import SwiftUI

/// The one place that decides how a diagnostic looks, so the editor underline, the status chip, the Problems
/// tab and its badge agree.
extension PineDiagnosticSeverity {
    var nsColor: NSColor {
        switch self {
        case .error: .systemRed
        case .warning: .systemYellow
        }
    }

    var color: Color { Color(nsColor: nsColor) }

    var symbol: String {
        switch self {
        case .error: "xmark.octagon.fill"
        case .warning: "exclamationmark.triangle.fill"
        }
    }
}
