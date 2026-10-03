import SwiftUI

/// The account's running journal: one line per thing that happened, newest first.
struct PaperJournalTable: View {
    @ObservedObject var store: PaperTradingStore

    var body: some View {
        if store.journal.isEmpty {
            PaperEmptyState(
                systemImage: "book.closed", title: "The journal is empty",
                message: "Fills, rejections and account changes are written here.")
        } else {
            Table(store.journal) {
                TableColumn("Time") { entry in
                    Text(PaperTimestamp.format(entry.timestamp))
                        .monospacedDigit().foregroundStyle(.secondary).lineLimit(1)
                }
                .width(min: 120, ideal: 140, max: 170)
                TableColumn("Entry") { entry in
                    Text(entry.message).lineLimit(2).textSelection(.enabled)
                }
                .width(min: 240, ideal: 480)
            }
            .portfolioTableChrome(inset: 12)
            .padding(.vertical, 8)
        }
    }
}
