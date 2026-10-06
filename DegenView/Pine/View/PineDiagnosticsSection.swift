import SwiftUI

/// A script's compile and runtime diagnostics in the same collapsible box as its alerts: severity
/// and counts in the header, the rows when open. Opens by itself while there is an error, since
/// that is what the user came to read; a warnings-only report waits to be opened.
struct PineDiagnosticsSection: View {
    let diagnostics: [PineDiagnostic]
    @State private var isExpanded: Bool
    @State private var copied = false

    init(diagnostics: [PineDiagnostic]) {
        self.diagnostics = diagnostics
        _isExpanded = State(initialValue: Self.startsExpanded(diagnostics))
    }

    struct Summary: Equatable {
        var errors = 0
        var warnings = 0
    }

    static func summary(for diagnostics: [PineDiagnostic]) -> Summary {
        diagnostics.reduce(into: Summary()) { summary, diagnostic in
            switch diagnostic.severity {
            case .error: summary.errors += 1
            case .warning: summary.warnings += 1
            }
        }
    }

    static func startsExpanded(_ diagnostics: [PineDiagnostic]) -> Bool { summary(for: diagnostics).errors > 0 }

    var body: some View {
        let summary = Self.summary(for: diagnostics)
        DisclosureGroup(isExpanded: $isExpanded) {
            PineDiagnosticsListView(diagnostics: diagnostics, maxHeight: 140, showsHeader: false)
                .padding(.top, 6)
        } label: {
            header(summary)
        }
        .font(.caption)
        .animation(.snappy, value: isExpanded)
        .pineReportBox()
        // New errors while editing open the box; closing it stays the user's call.
        .onChange(of: summary.errors > 0) { _, hasErrors in
            if hasErrors { isExpanded = true }
        }
    }

    // MARK: - Header

    private func header(_ summary: Summary) -> some View {
        let severity: PineDiagnosticSeverity = summary.errors > 0 ? .error : .warning
        return HStack(spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: severity.symbol).foregroundStyle(severity.color)
                Text("Diagnostics").font(.caption.weight(.semibold))
                if summary.errors > 0 {
                    PineCountChip(
                        text: PineCountChip.plural(summary.errors, "error"), tint: PineDiagnosticSeverity.error.color)
                }
                if summary.warnings > 0 {
                    PineCountChip(
                        text: PineCountChip.plural(summary.warnings, "warning"),
                        tint: PineDiagnosticSeverity.warning.color)
                }
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(accessibilitySummary(summary))
            Spacer(minLength: 8)
            copyButton
        }
    }

    private var copyButton: some View {
        Button {
            PineDiagnosticsListView.copyToPasteboard(diagnostics)
            copied = true
            Task {
                try? await Task.sleep(for: .seconds(2))
                copied = false
            }
        } label: {
            Label(
                copied ? "Copied" : PineDiagnosticsListView.copyTitle(for: diagnostics),
                systemImage: copied ? "checkmark" : "doc.on.doc")
        }
        .buttonStyle(.borderless)
        .font(.caption)
    }

    private func accessibilitySummary(_ summary: Summary) -> String {
        let parts = [(summary.errors, "error"), (summary.warnings, "warning")]
            .filter { $0.0 > 0 }
            .map { PineCountChip.plural($0.0, $0.1) }
        return (["Diagnostics"] + parts).joined(separator: ", ")
    }
}
