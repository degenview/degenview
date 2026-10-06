import SwiftUI

/// Create a paper account, or reset the selected one. A reset keeps the account but wipes its
/// positions, orders, fills and journal, so it opens on a red warning and starts from the account's
/// current settings rather than defaults.
struct PaperAccountConfigurationSheet: View {
    enum Mode {
        case create
        case reset(PaperAccount)
    }

    private enum CommissionKind: String, CaseIterable, Identifiable {
        case none = "None"
        case fixed = "Fixed"
        case percentage = "Percent"
        case perContract = "Per contract"
        var id: String { rawValue }
    }

    private static let leveragePresets: [Decimal] = [1, 2, 5, 10, 20, 50, 100]

    let mode: Mode
    let completion: (String, PaperCurrency, Decimal, PaperAccountSettings) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var balanceText: String
    @State private var currency: PaperCurrency
    @State private var leverageText: String
    @State private var commissionKind: CommissionKind
    @State private var commissionText: String

    init(mode: Mode, completion: @escaping (String, PaperCurrency, Decimal, PaperAccountSettings) -> Void) {
        self.mode = mode
        self.completion = completion
        switch mode {
        case .create:
            _name = State(initialValue: "Paper Trading")
            _balanceText = State(initialValue: "100000")
            _currency = State(initialValue: .USD)
            _leverageText = State(initialValue: "1")
            _commissionKind = State(initialValue: .none)
            _commissionText = State(initialValue: "0")
        case .reset(let account):
            _name = State(initialValue: account.name)
            _balanceText = State(initialValue: NSDecimalNumber(decimal: account.initialBalance).stringValue)
            _currency = State(initialValue: account.baseCurrency)
            _leverageText = State(initialValue: NSDecimalNumber(decimal: account.settings.leverage.crypto).stringValue)
            switch account.settings.commission {
            case .none:
                _commissionKind = State(initialValue: .none)
                _commissionText = State(initialValue: "0")
            case .fixedPerOrder(let value):
                _commissionKind = State(initialValue: .fixed)
                _commissionText = State(initialValue: NSDecimalNumber(decimal: value).stringValue)
            case .percentage(let value):
                _commissionKind = State(initialValue: .percentage)
                _commissionText = State(initialValue: NSDecimalNumber(decimal: value).stringValue)
            case .perContract(let value):
                _commissionKind = State(initialValue: .perContract)
                _commissionText = State(initialValue: NSDecimalNumber(decimal: value).stringValue)
            }
        }
    }

    private var isReset: Bool {
        if case .reset = mode { return true }
        return false
    }

    // MARK: Validation

    private var balance: Decimal? { PaperDecimalInput.parse(balanceText) }
    private var leverage: Decimal? { PaperDecimalInput.parse(leverageText) }
    private var commissionValue: Decimal? { PaperDecimalInput.parse(commissionText) }

    private var balanceError: String? {
        guard let balance else { return "Enter a number." }
        return balance > 0 ? nil : "Must be greater than zero."
    }

    private var leverageError: String? {
        guard let leverage else { return "Enter a number." }
        return (1...1000).contains(leverage) ? nil : "Must be between 1× and 1000×."
    }

    private var commissionError: String? {
        guard commissionKind != .none else { return nil }
        guard let commissionValue else { return "Enter a number." }
        return commissionValue >= 0 ? nil : "Can't be negative."
    }

    private var canSubmit: Bool {
        !name.trimmingCharacters(in: .whitespaces).isEmpty
            && balanceError == nil && leverageError == nil && commissionError == nil
    }

    // MARK: Body

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SheetHeader(
                title: isReset ? "Reset Paper Account" : "New Paper Account",
                subtitle: isReset ? "Start over with a fresh balance." : "Practice with simulated money."
            ) {
                badge
            }
            if isReset {
                NoticeCard(
                    systemImage: "exclamationmark.triangle.fill", tint: .red,
                    title: "This can't be undone",
                    detail: "Every position, order, fill, history and journal entry in this account is removed.")
            }
            PaperFormField(label: "Account name") {
                PaperTextField(placeholder: "Paper Trading", text: $name, isMonospaced: false)
                    .disabled(isReset)
                    .opacity(isReset ? 0.6 : 1)
            }
            HStack(alignment: .top, spacing: 12) {
                PaperFormField(label: "Starting balance", error: visibleBalanceError) {
                    PaperTextField(
                        placeholder: "100000", text: $balanceText, suffix: currency.rawValue,
                        isError: balanceError != nil && !balanceText.isEmpty)
                }
                PaperFormField(label: "Currency") {
                    Picker("Currency", selection: $currency) {
                        ForEach(PaperCurrency.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .labelsHidden()
                    .frame(width: 96)
                }
            }
            leverageField
            commissionField
            if isReset, let balance {
                Label(
                    "Balance resets to \(PaperTradingFormatter.money(balance, currency: currency)).",
                    systemImage: "arrow.counterclockwise"
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(isReset ? "Reset Account" : "Create Account") { submit() }
                    .buttonStyle(.borderedProminent)
                    .tint(isReset ? .red : .accentColor)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    /// The balance error, once something has been typed.
    private var visibleBalanceError: String? { balanceText.isEmpty ? nil : balanceError }

    private var badge: some View {
        let tint: Color = isReset ? .red : .accentColor
        return Image(systemName: isReset ? "arrow.counterclockwise" : "plus.circle")
            .font(.system(size: 19, weight: .semibold))
            .foregroundStyle(tint)
            .frame(width: 46, height: 46)
            .background(tint.opacity(0.14), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    // MARK: Leverage

    private var leverageField: some View {
        PaperFormField(
            label: "Leverage", error: leverageText.isEmpty ? nil : leverageError,
            hint: "Applies to every market. 1× means no borrowing."
        ) {
            HStack(spacing: 8) {
                PaperTextField(
                    placeholder: "1", text: $leverageText, suffix: "×",
                    isError: leverageError != nil && !leverageText.isEmpty
                )
                .frame(width: 110)
                ForEach(Self.leveragePresets, id: \.self) { preset in
                    let selected = leverage == preset
                    Button("\(NSDecimalNumber(decimal: preset).intValue)×") {
                        leverageText = NSDecimalNumber(decimal: preset).stringValue
                    }
                    .buttonStyle(.plain)
                    .font(.caption.weight(.semibold).monospacedDigit())
                    .foregroundStyle(selected ? Color.accentColor : .secondary)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 5)
                    .background(
                        selected ? Color.accentColor.opacity(0.14) : Color.primary.opacity(0.05),
                        in: Capsule())
                }
            }
        }
    }

    // MARK: Commission

    private var commissionField: some View {
        PaperFormField(label: "Commission", error: commissionKind == .none ? nil : commissionError) {
            VStack(alignment: .leading, spacing: 8) {
                Picker("Commission", selection: $commissionKind) {
                    ForEach(CommissionKind.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                if commissionKind != .none {
                    PaperTextField(
                        placeholder: "0", text: $commissionText, suffix: commissionUnit,
                        isError: commissionError != nil
                    )
                }
            }
        }
    }

    private var commissionUnit: String {
        switch commissionKind {
        case .none: ""
        case .fixed: "\(currency.rawValue) per order"
        case .percentage: "% of value"
        case .perContract: "\(currency.rawValue) per contract"
        }
    }

    // MARK: Submit

    private func submit() {
        guard canSubmit, let balance, let leverage else { return }
        let fee = commissionValue ?? 0
        var settings: PaperAccountSettings
        if case .reset(let account) = mode { settings = account.settings } else { settings = PaperAccountSettings() }
        settings.commission =
            switch commissionKind {
            case .none: .none
            case .fixed: .fixedPerOrder(fee)
            case .percentage: .percentage(fee)
            case .perContract: .perContract(fee)
            }
        settings.leverage = .init(
            stocks: leverage, crypto: leverage, forex: leverage, futures: leverage, prediction: leverage)
        completion(name.trimmingCharacters(in: .whitespaces), currency, balance, settings)
        dismiss()
    }
}
