import SwiftUI

/// Add, edit or duplicate one portfolio transaction. Everything it computes — the price/total
/// link, the validation, the summary — lives in `TransactionEditorDraft`.
struct TransactionEditorSheet: View {
    @ObservedObject var store: PortfolioStore
    let context: TransactionEditorContext
    @Environment(\.dismiss) private var dismiss
    @StateObject private var info = PortfolioAssetInfoViewModel()
    @State private var draft: TransactionEditorDraft
    @State private var submitting = false
    @State private var saveError: String?
    /// The asset's price now, in the transaction's currency; nil until it arrives or when there is none.
    @State private var marketPrice: Decimal?

    init(store: PortfolioStore, context: TransactionEditorContext) {
        self.store = store
        self.context = context
        _draft = State(initialValue: TransactionEditorDraft(transaction: context.transaction))
    }

    private var asset: PortfolioAsset { context.transaction.asset }
    private var priceCurrency: PortfolioCurrency { context.transaction.priceCurrency }

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
        .task { info.load([asset]) }
        .task { await loadMarketPrice() }
        .onChange(of: draft) { _, _ in saveError = nil }
    }

    /// Looks the price up once. A new transaction opens with it in the price field; an edit or a
    /// duplicate keeps the price it came with and only offers it through the field's Market button.
    private func loadMarketPrice() async {
        let quote = await MarketQuoteCoordinator.shared.quote(for: asset)
        guard let quote, quote.isFresh,
            let rate = await FXRateService.shared.rate(from: quote.currency, to: priceCurrency)
        else { return }
        let price = quote.price * rate
        marketPrice = price
        if context.mode == .new { draft.prefillPrice(price) }
    }

    // MARK: Form

    private var form: some View {
        let issues = draft.issues
        return VStack(alignment: .leading, spacing: 16) {
            header
            typeSelector
            quantityField(issues)
            if draft.usesPrice { priceFields(issues) }
            HStack(alignment: .top, spacing: 12) {
                feeField(issues).frame(width: 150)
                dateField
            }
            notesField
            if let summary = draft.summary {
                TransactionSummaryCard(summary: summary, currency: priceCurrency)
            }
            if let saveError {
                NoticeCard(
                    systemImage: "exclamationmark.triangle.fill", tint: .red,
                    title: "Transaction not saved", detail: saveError)
            }
        }
        .padding(24)
        .animation(.easeOut(duration: 0.15), value: draft.usesPrice)
    }

    private var title: String {
        switch context.mode {
        case .new: "Add Transaction"
        case .edit: "Edit Transaction"
        case .duplicate: "Duplicate Transaction"
        }
    }

    private var header: some View {
        SheetHeader(title: title, subtitle: "\(asset.displayTicker) · \(asset.source.displayName)") {
            AlertAssetIcon(asset: asset, info: info, size: 40)
                .frame(width: 46, height: 46)
        }
    }

    // MARK: Type

    private var typeSelector: some View {
        VStack(alignment: .leading, spacing: 10) {
            IconTabBar(
                items: TransactionEditorDraft.Category.allCases.map {
                    IconTabBar<TransactionEditorDraft.Category>.Item(
                        value: $0, title: $0.rawValue, systemImage: $0.systemImage)
                },
                selection: Binding(
                    get: { draft.category },
                    set: { draft.select($0) }),
                isCompact: true)
            let types = draft.category.types
            if types.count > 1 {
                HStack(spacing: 6) {
                    ForEach(types) { type in
                        TransactionTypeChip(type: type, isSelected: draft.type == type) {
                            withAnimation(.easeOut(duration: 0.15)) { draft.type = type }
                        }
                    }
                    Spacer(minLength: 0)
                }
                .transition(.opacity)
            }
        }
        .animation(.easeOut(duration: 0.15), value: draft.category)
    }

    // MARK: Amounts

    private func quantityField(_ issues: [TransactionEditorDraft.Field: String]) -> some View {
        PaperFormField(label: "Quantity", error: issues[.quantity]) {
            PaperTextField(
                placeholder: "0", text: $draft.quantityText, suffix: asset.displayTicker,
                isError: issues[.quantity] != nil, autofocus: true)
        }
    }

    private func priceFields(_ issues: [TransactionEditorDraft.Field: String]) -> some View {
        HStack(alignment: .top, spacing: 12) {
            PaperFormField(
                label: "Price per \(asset.displayTicker)", error: issues[.price],
                hint: draft.requiresPrice ? "Required" : "Optional"
            ) {
                PaperTextField(
                    placeholder: "0.00",
                    text: Binding(get: { draft.displayedPrice }, set: { draft.setPrice($0) }),
                    suffix: priceCurrency.rawValue, isError: issues[.price] != nil,
                    accessoryTitle: marketPrice == nil ? nil : "Market",
                    accessoryAction: {
                        if let marketPrice { draft.setPrice(draft.text(forMarketPrice: marketPrice)) }
                    })
            }
            PaperFormField(label: "Total", error: issues[.total]) {
                PaperTextField(
                    placeholder: "0.00",
                    text: Binding(get: { draft.displayedTotal }, set: { draft.setTotal($0) }),
                    suffix: priceCurrency.rawValue, isError: issues[.total] != nil)
            }
        }
    }

    private func feeField(_ issues: [TransactionEditorDraft.Field: String]) -> some View {
        PaperFormField(label: "Fee", error: issues[.fee]) {
            PaperTextField(
                placeholder: "0", text: $draft.feeText, suffix: context.transaction.feeCurrency.rawValue,
                isError: issues[.fee] != nil)
        }
    }

    private var dateField: some View {
        PaperFormField(label: "Date & time") {
            DatePicker("Date and time", selection: $draft.date)
                .datePickerStyle(.compact)
                .labelsHidden()
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(Color(nsColor: .separatorColor).opacity(0.6))
                }
        }
    }

    private var notesField: some View {
        PaperFormField(label: "Notes") {
            TransactionNotesField(text: $draft.notes)
        }
    }

    // MARK: Footer

    private var primaryTint: Color { draft.type == .adjustment ? .accentColor : draft.type.tint }

    private var footer: some View {
        HStack {
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
                .controlSize(.large)
            Spacer()
            Button(action: save) {
                HStack(spacing: 8) {
                    if submitting { ProgressView().controlSize(.small) }
                    Text(context.mode == .edit ? "Save Changes" : "Add Transaction")
                }
                .frame(minWidth: 130)
            }
            .buttonStyle(.borderedProminent)
            .tint(primaryTint)
            .controlSize(.large)
            .keyboardShortcut(.defaultAction)
            .disabled(!draft.canSubmit || submitting)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }

    private func save() {
        guard let transaction = draft.makeTransaction(from: context.transaction) else { return }
        submitting = true
        saveError = nil
        Task {
            let saved = context.mode == .edit ? await store.update(transaction) : await store.add(transaction)
            submitting = false
            if saved {
                dismiss()
                await store.refreshQuotes()
                await store.rebuildHistory()
            } else {
                // Shown here, beside the form; the dashboard's own alert is for its other errors.
                saveError = store.lastError ?? "The ledger rejected this transaction."
                store.lastError = nil
            }
        }
    }
}

/// One transaction type in the row under the category tabs; tinted with the type's own colour.
private struct TransactionTypeChip: View {
    let type: PortfolioTransactionType
    let isSelected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(type.rawValue)
                .font(.caption.weight(.semibold))
                .lineLimit(1)
                .foregroundStyle(isSelected ? type.tint : Color.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background((isSelected ? type.tint : Color.primary).opacity(isSelected ? 0.14 : 0.06), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

/// A multi-line field drawn like `PaperTextField`.
private struct TransactionNotesField: View {
    @Binding var text: String
    @FocusState private var isFocused: Bool

    var body: some View {
        TextField("Add a note (optional)", text: $text, axis: .vertical)
            .textFieldStyle(.plain)
            .lineLimit(2...4)
            .focused($isFocused)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(
                        isFocused ? Color.accentColor.opacity(0.8) : Color(nsColor: .separatorColor).opacity(0.6),
                        lineWidth: isFocused ? 1.5 : 1)
            }
            .contentShape(Rectangle())
            .onTapGesture { isFocused = true }
    }
}

/// What the transaction is worth, what it costs in fees, and the net figure.
private struct TransactionSummaryCard: View {
    let summary: TransactionEditorDraft.Summary
    let currency: PortfolioCurrency

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            row("Value", currency.format(summary.value))
            if summary.fee > 0 { row("Fee", currency.format(summary.fee)) }
            if let net = summary.net {
                Divider()
                row(net.label, currency.format(net.amount), isEmphasized: true)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.4)))
    }

    private func row(_ title: String, _ value: String, isEmphasized: Bool = false) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title)
                .font(.callout.weight(isEmphasized ? .semibold : .regular))
                .foregroundStyle(isEmphasized ? Color.primary : .secondary)
            Spacer(minLength: 12)
            Text(value)
                .font(.callout.weight(isEmphasized ? .semibold : .medium).monospacedDigit())
                .lineLimit(1)
        }
        .accessibilityElement(children: .combine)
    }
}
