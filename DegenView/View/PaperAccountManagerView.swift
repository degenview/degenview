import SwiftUI
import UniformTypeIdentifiers

enum PaperManagerTab: String, CaseIterable, Identifiable { case positions = "Positions", orders = "Orders", history = "History", accountHistory = "Account History", journal = "Trading Journal"; var id: String { rawValue } }

struct PaperAccountManagerView: View {
    @ObservedObject var store: PaperTradingStore
    @Binding var selectedTab: PaperManagerTab
    @Binding var showTradingOnCharts: Bool
    let onClose: () -> Void
    @State private var showCreate = false
    @State private var showReset = false

    var body: some View {
        VStack(spacing: 0) {
            header.padding(8)
            Divider()
            metrics.padding(.horizontal, 10).padding(.vertical, 7)
            Picker("Account section", selection: $selectedTab) {
                ForEach(PaperManagerTab.allCases) { Text($0.rawValue).tag($0) }
            }.pickerStyle(.segmented).padding(.horizontal, 10).padding(.bottom, 6)
            Divider()
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(.regularMaterial)
        .sheet(isPresented: $showCreate) { PaperAccountConfigurationSheet(title: "Create PAPER Account") { name, currency, balance, settings in
            Task { await store.createAccount(name: name, currency: currency, balance: balance, settings: settings) }
        }}
        .sheet(isPresented: $showReset) { PaperAccountConfigurationSheet(title: "Reset PAPER Account", destructive: true) { _, currency, balance, settings in
            Task { await store.reset(currency: currency, balance: balance, settings: settings) }
        }}
    }

    private var header: some View {
        HStack {
            Text("PAPER").font(.caption.bold()).padding(.horizontal, 7).padding(.vertical, 3).background(.blue, in: Capsule()).foregroundStyle(.white)
            Picker("Paper account", selection: Binding(get: { store.selectedAccount?.id }, set: { if let id = $0 { Task { await store.select(id) } } })) {
                ForEach(store.snapshot.accounts) { Text("\($0.name) — PAPER").tag(Optional($0.id)) }
            }.labelsHidden().frame(maxWidth: 260)
            Button { showCreate = true } label: { Image(systemName: "plus") }.help("Create paper account")
            Spacer()
            Button {
                showTradingOnCharts.toggle()
            } label: {
                Image(systemName: showTradingOnCharts ? "eye" : "eye.slash")
            }
            .buttonStyle(.plain)
            .accessibilityLabel(showTradingOnCharts ? "Hide trading controls on charts" : "Show trading controls on charts")
            .help(showTradingOnCharts ? "Hide Trading on Charts" : "Show Trading on Charts")
            Menu {
                Button("Export trading data…") { exportCSV() }
                Divider(); Button("Reset account…", role: .destructive) { showReset = true }
            } label: { Image(systemName: "ellipsis.circle") }.menuStyle(.borderlessButton)
            Button(action: onClose) { Image(systemName: "xmark") }.buttonStyle(.plain).accessibilityLabel("Close Paper Trading panel")
        }
    }

    private var metrics: some View {
        HStack(spacing: 22) {
            metric("Balance", store.metrics?.balance); metric("Equity", store.metrics?.equity)
            metric("Realized P&L", store.metrics?.realizedPnL); metric("Unrealized P&L", store.metrics?.unrealizedPnL)
            metric("Account Margin", store.metrics?.positionMargin); metric("Orders Margin", store.metrics?.ordersMargin)
            metric("Available Funds", store.metrics?.availableFunds)
            percentageMetric("Margin Buffer", store.metrics?.marginBuffer)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }

    private func metric(_ title: String, _ value: Decimal?) -> some View {
        metricLabel(title, value.map(money) ?? "—")
    }

    private func percentageMetric(_ title: String, _ value: Decimal?) -> some View {
        metricLabel(title, value.map { PaperTradingFormatter.percent($0) } ?? "—")
    }

    private func metricLabel(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.caption2).foregroundStyle(.secondary)
            Text(value).font(.caption.monospacedDigit())
        }
    }

    @ViewBuilder private var content: some View {
        switch selectedTab {
        case .positions:
            Table(store.positions) {
                TableColumn("Symbol") { Text($0.instrument.symbol) }
                TableColumn("Side") { Text($0.side.rawValue.uppercased()) }
                TableColumn("Quantity") { Text(quantity($0.quantity, instrument: $0.instrument)) }
                TableColumn("Average") { Text(price($0.averageEntryPrice, instrument: $0.instrument)) }
                TableColumn("Unrealized") { Text(money(unrealized($0))) }
                TableColumn("Realized") { Text(money($0.realizedGrossPnL - $0.commissions)) }
                TableColumn("Actions") { position in HStack { Button("Close") { Task { await store.close(position) } }; Button("Reverse") { Task { await store.reverse(position) } } } }
            }
        case .orders:
            Table(store.workingOrders) {
                TableColumn("Symbol") { Text($0.instrument.symbol) }; TableColumn("Side") { Text($0.side.rawValue.uppercased()) }
                TableColumn("Type") { Text($0.type.rawValue) }; TableColumn("Quantity") { Text(quantity($0.originalQuantity, instrument: $0.instrument)) }
                TableColumn("Filled") { Text(quantity($0.filledQuantity, instrument: $0.instrument)) }
                TableColumn("Limit") { order in Text(order.limitPrice.map { price($0, instrument: order.instrument) } ?? "—") }
                TableColumn("Stop") { order in Text(order.stopPrice.map { price($0, instrument: order.instrument) } ?? "—") }
                TableColumn("Status") { Text($0.status.rawValue) }
                TableColumn("") { order in Button { Task { await store.cancel(order.id) } } label: { Image(systemName: "xmark.circle") }.accessibilityLabel("Cancel \(order.side.rawValue) \(order.type.rawValue) order") }
            }
        case .history:
            Table(store.orderHistory) { TableColumn("Time") { Text($0.timestamp.formatted(date: .omitted, time: .standard)) }; TableColumn("Order ID") { Text($0.orderID.uuidString.prefix(8)) }; TableColumn("Symbol") { Text($0.order.instrument.symbol) }; TableColumn("Event") { Text($0.kind.rawValue) }; TableColumn("Details") { Text($0.message) } }
        case .accountHistory:
            Table(store.closedTrades) { TableColumn("Symbol") { Text($0.instrument.symbol) }; TableColumn("Side") { Text($0.side.rawValue) }; TableColumn("Quantity") { Text(quantity($0.quantity, instrument: $0.instrument)) }; TableColumn("Entry") { Text(price($0.entryPrice, instrument: $0.instrument)) }; TableColumn("Exit") { Text(price($0.exitPrice, instrument: $0.instrument)) }; TableColumn("Gross P&L") { Text(money($0.grossPnL)) }; TableColumn("Commission") { Text(money($0.commission)) }; TableColumn("Net P&L") { Text(money($0.netPnL)) } }
        case .journal:
            List(store.journal) { entry in HStack(alignment: .firstTextBaseline) { Text(entry.timestamp.formatted(date: .abbreviated, time: .standard)).font(.caption.monospacedDigit()).foregroundStyle(.secondary); Text(entry.message).textSelection(.enabled) } }
        }
    }

    private func unrealized(_ position: PaperPosition) -> Decimal { store.snapshot.quotes[position.instrument.key].flatMap { quote in let mark = position.signedQuantity >= 0 ? (quote.bid ?? quote.last) : (quote.ask ?? quote.last); return mark.map { (position.signedQuantity >= 0 ? $0 - position.averageEntryPrice : position.averageEntryPrice - $0) * position.quantity * position.instrument.pointValue } } ?? 0 }
    private func money(_ value: Decimal) -> String { PaperTradingFormatter.money(value, currency: store.selectedAccount?.baseCurrency ?? .USD) }
    private func price(_ value: Decimal, instrument: PaperInstrument) -> String { PaperTradingFormatter.price(value, instrument: instrument) }
    private func quantity(_ value: Decimal, instrument: PaperInstrument) -> String { PaperTradingFormatter.quantity(value, instrument: instrument) }

    private func exportCSV() {
        let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.canCreateDirectories = true; panel.prompt = "Export"
        guard panel.runModal() == .OK, let directory = panel.url else { return }
        PaperTradingCSVExporter.export(snapshot: store.snapshot, accountID: store.selectedAccount?.id, to: directory)
    }
}

struct PaperAccountConfigurationSheet: View {
    let title: String; var destructive = false
    let completion: (String, PaperCurrency, Decimal, PaperAccountSettings) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = "Paper Trading"; @State private var balance = "100000"; @State private var currency: PaperCurrency = .USD
    @State private var leverage = "1"; @State private var commissionType = 0; @State private var commissionValue = "0"

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(title).font(.headline); if destructive { Text("This permanently removes all positions, orders, fills, history, and journal entries for this paper account.").font(.caption).foregroundStyle(.red) }
            TextField("Account name", text: $name).disabled(destructive); TextField("Initial balance", text: $balance)
            Picker("Currency", selection: $currency) { ForEach(PaperCurrency.allCases) { Text($0.rawValue).tag($0) } }
            TextField("Leverage", text: $leverage)
            Picker("Commission", selection: $commissionType) { Text("None").tag(0); Text("Fixed per order").tag(1); Text("Percentage").tag(2); Text("Per contract").tag(3) }
            if commissionType != 0 { TextField("Commission value", text: $commissionValue) }
            HStack { Spacer(); Button("Cancel", role: .cancel) { dismiss() }; Button(destructive ? "Reset Account" : "Create Account", role: destructive ? .destructive : nil) { submit() }.keyboardShortcut(.defaultAction) }
        }.padding(20).frame(width: 420)
    }
    private func submit() {
        let amount = Decimal(string: balance) ?? 100_000, lev = max(1, Decimal(string: leverage) ?? 1), fee = Decimal(string: commissionValue) ?? 0
        let commission: PaperCommissionConfiguration = switch commissionType { case 1: .fixedPerOrder(fee); case 2: .percentage(fee); case 3: .perContract(fee); default: .none }
        var settings = PaperAccountSettings(); settings.commission = commission; settings.leverage = .init(stocks: lev, crypto: lev, forex: lev, futures: lev, prediction: lev)
        completion(name, currency, amount, settings); dismiss()
    }
}

enum PaperTradingCSVExporter {
    static func export(snapshot: PaperTradingSnapshot, accountID: UUID?, to directory: URL) {
        guard let accountID else { return }
        let iso = ISO8601DateFormatter()
        let orders = (["order_id,symbol,side,type,quantity,filled,status,created_at"] + snapshot.orders.filter { $0.accountID == accountID }.map { "\($0.id.uuidString),\(csv($0.instrument.symbol)),\($0.side.rawValue),\($0.type.rawValue),\($0.originalQuantity),\($0.filledQuantity),\($0.status.rawValue),\(iso.string(from: $0.createdAt))" }).joined(separator: "\n")
        let fills = (["fill_id,order_id,symbol,side,quantity,price,commission,price_source,timestamp"] + snapshot.fills.filter { $0.accountID == accountID }.map { "\($0.id.uuidString),\($0.orderID.uuidString),\(csv($0.instrument.symbol)),\($0.side.rawValue),\($0.quantity),\($0.price),\($0.commission),\($0.priceSource.rawValue),\(iso.string(from: $0.timestamp))" }).joined(separator: "\n")
        let trades = (["trade_id,symbol,side,quantity,entry_price,exit_price,gross_pnl,commission,net_pnl,entry_time,exit_time"] + snapshot.closedTrades.filter { $0.accountID == accountID }.map { "\($0.id.uuidString),\(csv($0.instrument.symbol)),\($0.side.rawValue),\($0.quantity),\($0.entryPrice),\($0.exitPrice),\($0.grossPnL),\($0.commission),\($0.netPnL),\(iso.string(from: $0.entryTimestamp)),\(iso.string(from: $0.exitTimestamp))" }).joined(separator: "\n")
        try? orders.write(to: directory.appendingPathComponent("paper-orders.csv"), atomically: true, encoding: .utf8)
        try? fills.write(to: directory.appendingPathComponent("paper-fills.csv"), atomically: true, encoding: .utf8)
        try? trades.write(to: directory.appendingPathComponent("paper-trades.csv"), atomically: true, encoding: .utf8)
    }
    private static func csv(_ value: String) -> String { "\"\(value.replacingOccurrences(of: "\"", with: "\"\""))\"" }
}
