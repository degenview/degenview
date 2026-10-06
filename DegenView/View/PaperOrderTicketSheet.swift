import SwiftUI

/// The order ticket: opened from a chart's BUY / SELL buttons. Everything it computes — the
/// summary, the validation, the quantity shortcuts — lives in `PaperOrderTicketDraft`.
struct PaperOrderTicketSheet: View {
    @ObservedObject var store: PaperTradingStore
    let instrument: PaperInstrument
    let referencePrice: Decimal?
    let initialSide: PaperOrderSide
    @Environment(\.dismiss) private var dismiss

    @State private var side: PaperOrderSide
    @State private var type: PaperOrderType = .market
    @State private var quantityText: String
    @State private var limitPriceText: String
    @State private var stopPriceText: String
    @State private var takeProfitText = ""
    @State private var stopLossText = ""
    @State private var timeInForce: PaperTimeInForce = .goodTilCanceled
    @State private var showsProtection = false
    @State private var submitting = false

    init(store: PaperTradingStore, instrument: PaperInstrument, referencePrice: Decimal?, initialSide: PaperOrderSide) {
        self.store = store
        self.instrument = instrument
        self.referencePrice = referencePrice
        self.initialSide = initialSide
        _side = State(initialValue: initialSide)
        // Whole-unit instruments (stocks) start at one share; fractional ones start empty, where
        // a default of "1" would be a full coin.
        _quantityText = State(initialValue: instrument.quantityIncrement >= 1 ? "1" : "")
        let priceText = referencePrice.map { PaperDecimalInput.text(Self.roundedToTick($0, instrument)) } ?? ""
        _limitPriceText = State(initialValue: priceText)
        _stopPriceText = State(initialValue: priceText)
    }

    var body: some View {
        VStack(spacing: 0) {
            ViewThatFits(in: .vertical) {
                form
                ScrollView { form }.scrollBounceBehavior(.basedOnSize)
            }
            Divider()
            footer
        }
        .frame(width: 460)
        .onAppear { store.clearError() }
        .onDisappear { store.clearError() }
    }

    // MARK: Draft

    private var draft: PaperOrderTicketDraft {
        var draft = PaperOrderTicketDraft(instrument: instrument)
        let quote = store.snapshot.quotes[instrument.key]
        draft.bid = quote?.bid
        draft.ask = quote?.ask
        draft.last = quote?.last ?? referencePrice
        draft.availableFunds = store.metrics?.availableFunds
        if let account = store.selectedAccount {
            draft.leverage = account.settings.leverage.leverage(for: instrument.assetClass)
            draft.commission = account.settings.commission
        }
        draft.existingSignedQuantity =
            store.positions.first { $0.instrument.key == instrument.key }?.signedQuantity ?? 0
        draft.side = side
        draft.type = type
        draft.timeInForce = timeInForce
        draft.quantityText = quantityText
        draft.limitPriceText = limitPriceText
        draft.stopPriceText = stopPriceText
        draft.takeProfitText = takeProfitText
        draft.stopLossText = stopLossText
        return draft
    }

    private static func roundedToTick(_ value: Decimal, _ instrument: PaperInstrument) -> Decimal {
        guard instrument.tickSize > 0 else { return value }
        return Decimal.rounded(value / instrument.tickSize, scale: 0) * instrument.tickSize
    }

    // MARK: Form

    private var form: some View {
        let draft = draft
        return VStack(alignment: .leading, spacing: 16) {
            header
            PaperQuoteStrip(draft: draft)
            PaperSideSelector(selection: $side)
            IconTabBar(
                items: PaperOrderType.allCases.map {
                    IconTabBar<PaperOrderType>.Item(value: $0, title: $0.label, systemImage: $0.systemImage)
                },
                selection: $type, isCompact: true)
            quantityField(draft)
            priceFields(draft)
            protection(draft)
            if type != .market { timeInForceField }
            PaperOrderSummaryCard(draft: draft, currency: store.accountCurrency)
            if let error = store.lastError {
                NoticeCard(
                    systemImage: "exclamationmark.triangle.fill", tint: .red,
                    title: "Order not placed", detail: error)
            }
        }
        .padding(24)
    }

    private var header: some View {
        SheetHeader(title: instrument.displayName, subtitle: "\(instrument.source.displayName) · Paper order") {
            PaperInstrumentIcon(instrument: instrument, size: 40, showsSource: true)
                .frame(width: 46, height: 46)
        }
        .overlay(alignment: .topTrailing) { PaperBadge() }
    }

    // MARK: Quantity

    private func quantityField(_ draft: PaperOrderTicketDraft) -> some View {
        PaperFormField(
            label: "Quantity", error: draft.issues[.quantity],
            hint: type == .market ? "Fills immediately at the best available price." : nil
        ) {
            VStack(alignment: .leading, spacing: 8) {
                PaperTextField(
                    placeholder: "0", text: $quantityText, suffix: instrument.baseSymbol,
                    isError: draft.issues[.quantity] != nil)
                HStack(spacing: 6) {
                    ForEach([(25, "25%"), (50, "50%"), (75, "75%"), (100, "Max")], id: \.0) { percent, title in
                        quickFill(title, draft: draft, fraction: Decimal(percent) / 100)
                    }
                    Spacer(minLength: 0)
                    Text("of available funds").font(.caption).foregroundStyle(.tertiary)
                }
            }
        }
    }

    private func quickFill(_ title: String, draft: PaperOrderTicketDraft, fraction: Decimal) -> some View {
        let quantity = draft.quantity(forFundsFraction: fraction)
        return Button(title) {
            if let quantity { quantityText = PaperDecimalInput.text(quantity) }
        }
        .buttonStyle(.plain)
        .font(.caption.weight(.semibold).monospacedDigit())
        .foregroundStyle(quantity == nil ? Color.secondary.opacity(0.5) : Color.secondary)
        .padding(.horizontal, 9)
        .padding(.vertical, 4)
        .background(Color.primary.opacity(0.06), in: Capsule())
        .disabled(quantity == nil)
    }

    // MARK: Prices

    @ViewBuilder
    private func priceFields(_ draft: PaperOrderTicketDraft) -> some View {
        if type != .market {
            HStack(alignment: .top, spacing: 12) {
                if draft.usesStopPrice {
                    PaperFormField(
                        label: type == .stopLimit ? "Stop trigger" : "Stop price", error: draft.issues[.stopPrice]
                    ) {
                        priceField($stopPriceText, draft: draft, isError: draft.issues[.stopPrice] != nil)
                    }
                }
                if draft.usesLimitPrice {
                    PaperFormField(label: "Limit price", error: draft.issues[.limitPrice]) {
                        priceField($limitPriceText, draft: draft, isError: draft.issues[.limitPrice] != nil)
                    }
                }
            }
        }
    }

    private func priceField(_ text: Binding<String>, draft: PaperOrderTicketDraft, isError: Bool) -> some View {
        PaperTextField(
            placeholder: "0", text: text, suffix: instrument.quoteCurrency.rawValue, isError: isError,
            accessoryTitle: draft.marketPrice == nil ? nil : "Market",
            accessoryAction: {
                if let price = draft.marketPrice { text.wrappedValue = PaperDecimalInput.text(price) }
            })
    }

    // MARK: Protection

    private func protection(_ draft: PaperOrderTicketDraft) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Button {
                withAnimation(.easeOut(duration: 0.15)) { showsProtection.toggle() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(showsProtection ? 90 : 0))
                    Text("Take profit & stop loss").font(.subheadline.weight(.medium))
                    Text("Optional").font(.caption).foregroundStyle(.tertiary)
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Take profit and stop loss")
            .accessibilityValue(showsProtection ? "Expanded" : "Collapsed")
            if showsProtection {
                HStack(alignment: .top, spacing: 12) {
                    PaperFormField(label: "Take profit", error: draft.issues[.takeProfit]) {
                        PaperTextField(
                            placeholder: "Price", text: $takeProfitText, suffix: instrument.quoteCurrency.rawValue,
                            isError: draft.issues[.takeProfit] != nil)
                    }
                    PaperFormField(label: "Stop loss", error: draft.issues[.stopLoss]) {
                        PaperTextField(
                            placeholder: "Price", text: $stopLossText, suffix: instrument.quoteCurrency.rawValue,
                            isError: draft.issues[.stopLoss] != nil)
                    }
                }
            }
        }
    }

    private var timeInForceField: some View {
        PaperFormField(label: "Time in force") {
            Picker("Time in force", selection: $timeInForce) {
                ForEach(PaperTimeInForce.allCases) { Text($0.label).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
        }
    }

    // MARK: Footer

    private var footer: some View {
        let draft = draft
        return HStack {
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
            Spacer()
            Button {
                submit()
            } label: {
                HStack(spacing: 8) {
                    if submitting { ProgressView().controlSize(.small) }
                    Text(submitTitle(draft))
                }
                .frame(minWidth: 150)
            }
            .buttonStyle(.borderedProminent)
            .tint(side.tint)
            .controlSize(.large)
            .keyboardShortcut(.defaultAction)
            .disabled(!draft.canSubmit || submitting)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }

    /// "Buy 0.5 BTC" once there is a quantity; "Buy BTC" before.
    private func submitTitle(_ draft: PaperOrderTicketDraft) -> String {
        guard let quantity = draft.quantity, quantity > 0 else { return "\(side.label) \(instrument.baseSymbol)" }
        let formatted = PaperTradingFormatter.quantity(quantity, instrument: instrument)
        return "\(side.label) \(formatted) \(instrument.baseSymbol)"
    }

    private func submit() {
        guard let accountID = store.selectedAccount?.id, let request = draft.request(accountID: accountID) else {
            return
        }
        submitting = true
        Task {
            let success = await store.submit(request)
            submitting = false
            if success { dismiss() }
        }
    }
}
