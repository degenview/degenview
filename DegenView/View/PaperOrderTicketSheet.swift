import SwiftUI

struct PaperOrderTicketSheet: View {
    @ObservedObject var store: PaperTradingStore
    let instrument: PaperInstrument
    let referencePrice: Decimal?
    let initialSide: PaperOrderSide
    @Environment(\.dismiss) private var dismiss

    @State private var side: PaperOrderSide
    @State private var type: PaperOrderType = .market
    @State private var quantity = "1"
    @State private var limitPrice = ""
    @State private var stopPrice = ""
    @State private var takeProfit = ""
    @State private var stopLoss = ""
    @State private var timeInForce: PaperTimeInForce = .goodTilCanceled
    @State private var submitting = false

    init(store: PaperTradingStore, instrument: PaperInstrument, referencePrice: Decimal?, initialSide: PaperOrderSide) {
        self.store = store
        self.instrument = instrument
        self.referencePrice = referencePrice
        self.initialSide = initialSide
        _side = State(initialValue: initialSide)
        let text = referencePrice.map { NSDecimalNumber(decimal: $0).stringValue } ?? ""
        _limitPrice = State(initialValue: text)
        _stopPrice = State(initialValue: text)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Label("PAPER", systemImage: "doc.text.fill").font(.headline).foregroundStyle(.blue)
                Text(instrument.displayName).font(.headline).lineLimit(1)
                Spacer()
            }
            Picker("Side", selection: $side) {
                Text("Buy").tag(PaperOrderSide.buy)
                Text("Sell").tag(PaperOrderSide.sell)
            }.pickerStyle(.segmented)
            Picker("Order Type", selection: $type) {
                ForEach(PaperOrderType.allCases) { Text(label($0)).tag($0) }
            }
            TextField("Quantity", text: $quantity).accessibilityLabel("Paper order quantity")
            if type == .limit || type == .stopLimit {
                TextField("Limit price", text: $limitPrice).accessibilityLabel("Paper limit price")
            }
            if type == .stop || type == .stopLimit {
                TextField("Stop price", text: $stopPrice).accessibilityLabel("Paper stop price")
            }
            DisclosureGroup("Protection (optional)") {
                TextField("Take profit", text: $takeProfit)
                TextField("Stop loss", text: $stopLoss)
            }
            Picker("Time in Force", selection: $timeInForce) {
                ForEach(PaperTimeInForce.allCases) { Text($0.rawValue).tag($0) }
            }
            orderInfo
            if let error = store.lastError {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button(submitting ? "Submitting…" : "Submit PAPER Order") { submit() }
                    .keyboardShortcut(.defaultAction).disabled(!isValid || submitting)
            }
        }
        .padding(20).frame(width: 420)
        .onDisappear { store.clearError() }
    }

    private var orderInfo: some View {
        GroupBox("Order Info") {
            Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 5) {
                GridRow {
                    Text("Trade value")
                    Text(money(tradeValue))
                }
                GridRow {
                    Text("Required margin")
                    Text(money(requiredMargin))
                }
                GridRow {
                    Text("Available funds")
                    Text(money(store.metrics?.availableFunds))
                }
                GridRow {
                    Text("Execution data")
                    Text("Bid/ask preferred; last-price fallback active").foregroundStyle(.secondary)
                }
            }.font(.caption).frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var parsedQuantity: Decimal? { Decimal(string: quantity, locale: Locale(identifier: "en_US_POSIX")) }
    private var selectedPrice: Decimal? {
        switch type {
        case .market: referencePrice
        case .limit, .stopLimit: Decimal(string: limitPrice, locale: Locale(identifier: "en_US_POSIX"))
        case .stop: Decimal(string: stopPrice, locale: Locale(identifier: "en_US_POSIX"))
        }
    }
    private var tradeValue: Decimal? {
        guard let q = parsedQuantity, let p = selectedPrice else { return nil }
        return q * p * instrument.contractMultiplier
    }
    private var requiredMargin: Decimal? {
        guard let value = tradeValue, let account = store.selectedAccount else { return nil }
        return value / max(1, account.settings.leverage.leverage(for: instrument.assetClass))
    }
    private var isValid: Bool { (parsedQuantity ?? 0) > 0 && (type == .market || selectedPrice != nil) }

    private func submit() {
        guard let accountID = store.selectedAccount?.id, let quantity = parsedQuantity else { return }
        submitting = true
        let request = PaperOrderRequest(
            accountID: accountID, instrument: instrument, side: side, type: type,
            quantity: quantity,
            limitPrice: (type == .limit || type == .stopLimit)
                ? Decimal(string: limitPrice, locale: Locale(identifier: "en_US_POSIX")) : nil,
            stopPrice: (type == .stop || type == .stopLimit)
                ? Decimal(string: stopPrice, locale: Locale(identifier: "en_US_POSIX")) : nil,
            timeInForce: timeInForce,
            takeProfit: Decimal(string: takeProfit, locale: Locale(identifier: "en_US_POSIX")),
            stopLoss: Decimal(string: stopLoss, locale: Locale(identifier: "en_US_POSIX")))
        Task {
            let success = await store.submit(request)
            submitting = false
            if success { dismiss() }
        }
    }

    private func label(_ type: PaperOrderType) -> String {
        switch type {
        case .market: "Market"
        case .limit: "Limit"
        case .stop: "Stop"
        case .stopLimit: "Stop Limit"
        }
    }
    private func money(_ value: Decimal?) -> String {
        guard let value else { return "—" }
        return PaperTradingFormatter.money(value, currency: store.selectedAccount?.baseCurrency ?? .USD)
    }
}
