import SwiftUI

/// Paste or open a list of `SOURCE:SYMBOL` lines, then see exactly what went in and what did not.
struct WatchlistImportSheet: View {
    let listName: String
    let onImport: (String) -> WatchlistImportReport?

    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var report: WatchlistImportReport?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Import into \(listName)").font(.title3.weight(.semibold))

            if let report {
                resultView(report)
            } else {
                Text("One symbol per line as SOURCE:SYMBOL, for example BINANCE:BTCUSDT or COINBASE:ETH-USD. Lines starting with ### become sections. A single comma-separated line from TradingView is read too; markets DegenView has no provider for are skipped.")
                    .font(.callout).foregroundStyle(.secondary)
                TextEditor(text: $text)
                    .font(.system(.body, design: .monospaced))
                    .frame(minHeight: 180)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.quaternary))
                    .accessibilityLabel("Symbols to import")
            }

            HStack {
                if report == nil {
                    Button("Open File…") {
                        if let contents = WatchlistFileIO.chooseTextFile() { text = contents }
                    }
                }
                Spacer()
                if report == nil {
                    Button("Cancel", role: .cancel) { dismiss() }
                        .keyboardShortcut(.cancelAction)
                    Button("Import") { report = onImport(text) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                } else {
                    Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(20)
        .frame(width: 460, height: 380)
    }

    private func resultView(_ report: WatchlistImportReport) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(report.summary, systemImage: report.skipped.isEmpty ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                .foregroundStyle(report.skipped.isEmpty ? .green : .orange)
            if !report.skipped.isEmpty {
                Text("Skipped").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                List(report.skipped, id: \.line) { skipped in
                    VStack(alignment: .leading, spacing: 1) {
                        Text(skipped.text).font(.system(.caption, design: .monospaced)).lineLimit(1)
                        Text(skipped.reason).font(.caption2).foregroundStyle(.secondary)
                    }
                }
                .listStyle(.bordered)
            }
            Spacer(minLength: 0)
        }
    }
}
