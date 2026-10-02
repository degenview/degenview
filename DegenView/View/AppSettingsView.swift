import SwiftUI

enum SettingsTab: String, CaseIterable, Identifiable {
    case appearance
    case alpaca
    case coinMarketCap
    case notifications

    var id: Self { self }

    var title: String {
        switch self {
        case .appearance: "Appearance"
        case .alpaca: "Alpaca"
        case .coinMarketCap: "CoinMarketCap"
        case .notifications: "Notifications"
        }
    }

    var systemImage: String {
        switch self {
        case .appearance: "paintbrush.fill"
        case .alpaca: "chart.xyaxis.line"
        case .coinMarketCap: "gauge.with.dots.needle.50percent"
        case .notifications: "bell.badge.fill"
        }
    }

    /// The sidebar badge color, as in the system Settings app.
    var tint: Color {
        switch self {
        case .appearance: .indigo
        case .alpaca: .orange
        case .coinMarketCap: .blue
        case .notifications: .red
        }
    }
}

/// The app's Settings window: a sidebar of badge-and-title rows beside the selected page.
struct AppSettingsView: View {
    @AppStorage("settingsTab") private var selectedTab: SettingsTab = .appearance
    @AppStorage("appTheme") private var appTheme: AppTheme = .system

    var body: some View {
        HStack(spacing: 0) {
            List(SettingsTab.allCases, selection: $selectedTab) { tab in
                HStack(spacing: 10) {
                    Image(systemName: tab.systemImage)
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.white)
                        .frame(width: 24, height: 24)
                        .background(tab.tint.gradient, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                    Text(tab.title)
                }
                .padding(.vertical, 2)
                .tag(tab)
            }
            .listStyle(.sidebar)
            .frame(width: 200)

            Divider()

            selectedContent
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .frame(width: 800, height: 580)
        .preferredColorScheme(appTheme.colorScheme)
    }

    @ViewBuilder
    private var selectedContent: some View {
        switch selectedTab {
        case .appearance: AppearanceSettingsView()
        case .alpaca: AlpacaSettingsView()
        case .coinMarketCap: CoinMarketCapSettingsView()
        case .notifications: NotificationSettingsView()
        }
    }
}
