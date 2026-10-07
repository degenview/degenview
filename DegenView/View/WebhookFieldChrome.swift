import SwiftUI

/// The rounded field look shared by the webhook editor's inputs: a quiet fill, a hairline border that
/// turns accent when focused and red when the value is wrong.
struct WebhookFieldChrome: ViewModifier {
    var isFocused: Bool
    var isInvalid = false

    private var border: Color {
        if isInvalid { return .red.opacity(0.7) }
        return isFocused ? Color.accentColor.opacity(0.7) : Color.primary.opacity(0.12)
    }

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(border, lineWidth: 1))
    }
}

extension View {
    func webhookFieldChrome(isFocused: Bool, isInvalid: Bool = false) -> some View {
        modifier(WebhookFieldChrome(isFocused: isFocused, isInvalid: isInvalid))
    }
}
