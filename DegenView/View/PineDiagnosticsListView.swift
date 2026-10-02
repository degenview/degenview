import AppKit
import SwiftUI

/// Compile and runtime diagnostics as selectable monospaced rows, with a button that copies the
/// errors (or everything, when there are only warnings) to the pasteboard.
struct PineDiagnosticsListView: View {
    let diagnostics: [PineDiagnostic]
    /// Caps the list so a long report scrolls instead of growing its container; nil lets it fill.
    var maxHeight: CGFloat? = 90
    var showsHeader = true

    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if showsHeader {
                HStack {
                    Text("Diagnostics").font(.caption.weight(.semibold))
                    Spacer()
                    copyButton
                }
            }

            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Self.ordered(diagnostics)) { diagnostic in
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Image(systemName: diagnostic.severity.symbol)
                                .foregroundStyle(diagnostic.severity.color)
                            Text(Self.format(diagnostic))
                                .foregroundStyle(.primary)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .fixedSize(horizontal: false, vertical: true)
                                .textSelection(.enabled)
                        }
                        .font(.caption.monospaced())
                    }
                }
            }
            .frame(maxHeight: maxHeight)
        }
    }

    private var copyButton: some View {
        Button {
            copy()
        } label: {
            Label(copied ? "Copied" : copyTitle, systemImage: copied ? "checkmark" : "doc.on.doc")
        }
        .buttonStyle(.borderless)
        .font(.caption)
    }

    private var copyTitle: String {
        diagnostics.contains { $0.severity == .error } ? "Copy Errors" : "Copy Problems"
    }

    /// Errors first, then warnings; source order within each.
    static func ordered(_ diagnostics: [PineDiagnostic]) -> [PineDiagnostic] {
        diagnostics.enumerated()
            .sorted { lhs, rhs in
                if lhs.element.severity != rhs.element.severity { return lhs.element.severity == .error }
                return lhs.offset < rhs.offset
            }
            .map(\.element)
    }

    static func format(_ diagnostic: PineDiagnostic) -> String {
        "\(diagnostic.code) · \(diagnostic.range.start.line):\(diagnostic.range.start.column)  \(diagnostic.message)"
    }

    private func copy() {
        let errors = diagnostics.filter { $0.severity == .error }
        let text = (errors.isEmpty ? diagnostics : errors).map(Self.format).joined(separator: "\n")

        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        copied = true
        Task {
            try? await Task.sleep(for: .seconds(2))
            copied = false
        }
    }
}
