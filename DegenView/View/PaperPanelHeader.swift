import SwiftUI

/// The top of the panel: the PAPER badge, the account switcher, the section tabs and the panel's
/// own buttons. On a narrow window the tabs drop to a second row instead of being cut off.
struct PaperPanelHeader: View {
    @ObservedObject var store: PaperTradingStore
    @Binding var selectedTab: PaperManagerTab
    @Binding var showTradingOnCharts: Bool
    let onCreate: () -> Void
    let onExport: () -> Void
    let onReset: () -> Void
    let onClose: () -> Void

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                identity
                tabs
                Spacer(minLength: 8)
                actions
            }
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 12) {
                    identity
                    Spacer(minLength: 8)
                    actions
                }
                ScrollView(.horizontal, showsIndicators: false) { tabs }
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    // MARK: Identity

    private var identity: some View {
        HStack(spacing: 8) {
            PaperBadge()
            accountMenu
        }
    }

    private var accountMenu: some View {
        Menu {
            ForEach(store.snapshot.accounts) { account in
                Button {
                    Task { await store.select(account.id) }
                } label: {
                    if account.id == store.selectedAccount?.id {
                        Label(account.name, systemImage: "checkmark")
                    } else {
                        Text(account.name)
                    }
                }
            }
            Divider()
            Button("New Account…", action: onCreate)
        } label: {
            HStack(spacing: 6) {
                Text(store.selectedAccount?.name ?? "No account")
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                    .frame(maxWidth: 170, alignment: .leading)
                if let currency = store.selectedAccount?.baseCurrency {
                    Text(currency.rawValue)
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.secondary)
            }
            .fixedSize()
            .padding(.horizontal, 9)
            .padding(.vertical, 5)
            .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("Switch paper account")
        .accessibilityLabel("Paper account")
    }

    // MARK: Tabs

    private var tabs: some View {
        IconTabBar(
            items: PaperManagerTab.allCases.map { tab in
                IconTabBar<PaperManagerTab>.Item(
                    value: tab, title: tab.title, systemImage: tab.systemImage, count: count(for: tab))
            },
            selection: $selectedTab, isCompact: true)
    }

    private func count(for tab: PaperManagerTab) -> Int? {
        switch tab {
        case .positions: store.positions.count
        case .orders: store.workingOrders.count
        case .history, .accountHistory, .journal: nil
        }
    }

    // MARK: Actions

    private var actions: some View {
        HStack(spacing: 2) {
            PaperIconButton(
                systemImage: showTradingOnCharts ? "eye" : "eye.slash",
                label: showTradingOnCharts ? "Hide trading on charts" : "Show trading on charts",
                isActive: showTradingOnCharts
            ) {
                showTradingOnCharts.toggle()
            }
            Menu {
                Button("New Account…", action: onCreate)
                Divider()
                Button("Export Trading Data…", action: onExport)
                Button("Reset Account…", role: .destructive, action: onReset)
            } label: {
                PaperIconGlyph(systemImage: "ellipsis")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Account actions")
            .accessibilityLabel("Paper account actions")
            PaperIconButton(systemImage: "xmark", label: "Close Paper Trading panel", action: onClose)
        }
    }
}
