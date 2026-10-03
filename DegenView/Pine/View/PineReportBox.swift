import SwiftUI

/// The grey rounded box every report section sits in: strategy metrics, alerts, diagnostics.
struct PineReportBox: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(10)
            .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 8))
    }
}

extension View {
    func pineReportBox() -> some View { modifier(PineReportBox()) }
}
