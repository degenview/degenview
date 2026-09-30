import SwiftUI

/// A collapsible, scrollable list of the most recent `limit` items, newest first.
struct PineReportListSection<Item: Identifiable, Row: View>: View {
    let title: String
    let items: [Item]
    let maxHeight: CGFloat
    @ViewBuilder let row: (Item) -> Row

    static var limit: Int { 100 }

    var body: some View {
        DisclosureGroup("\(title) (\(items.count))") {
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(items.suffix(Self.limit).reversed()) { row($0) }
                }
            }
            .frame(maxHeight: maxHeight)
        }
        .font(.caption)
    }
}
