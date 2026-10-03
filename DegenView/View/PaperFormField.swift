import SwiftUI

/// A form row: a label that stays above the control once a value is typed, the control, then a
/// red error (or a quiet hint) underneath.
struct PaperFormField<Content: View>: View {
    let label: String
    var error: String?
    var hint: String?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(label).font(.caption.weight(.medium)).foregroundStyle(.secondary)
            content
            if let error {
                Text(error).font(.caption).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
            } else if let hint {
                Text(hint).font(.caption).foregroundStyle(.tertiary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
