import SwiftUI

/// Trigger history grouped by day, newest first.
struct AlertHistoryList: View {
    /// Already filtered by the search.
    let events: [AlertTriggerEvent]
    @ObservedObject var info: PortfolioAssetInfoViewModel
    let rules: [UUID: PriceAlert]

    var body: some View {
        AlertCardScroll {
            ForEach(AlertDayGrouping.sections(events, date: \.timestamp)) { section in
                Section {
                    ForEach(section.items) { event in
                        AlertHistoryRow(event: event, info: info, rule: rules[event.alertID])
                    }
                } header: {
                    AlertDayHeader(title: section.title, count: section.items.count)
                }
            }
        }
    }
}
