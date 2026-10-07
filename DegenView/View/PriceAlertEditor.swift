import SwiftUI

/// Create or edit one price alert: what to watch for, at what level, how often, and a note.
struct PriceAlertEditor: View {
    private enum Direction: String, CaseIterable {
        case crossesAbove = "Above"
        case crossesBelow = "Below"
        case rises = "Rises"
        case falls = "Falls"

        var systemImage: String {
            switch self {
            case .crossesAbove: "arrow.up.right"
            case .crossesBelow: "arrow.down.right"
            case .rises: "arrow.up"
            case .falls: "arrow.down"
            }
        }

        var isPercent: Bool { self == .rises || self == .falls }

        var blurb: String {
            switch self {
            case .crossesAbove: "Notify me when the price crosses above a level."
            case .crossesBelow: "Notify me when the price crosses below a level."
            case .rises: "Notify me when the price climbs by a percentage from now."
            case .falls: "Notify me when the price drops by a percentage from now."
            }
        }
    }

    let asset: PortfolioAsset
    let existing: PriceAlert?
    @Environment(\.dismiss) private var dismiss
    @StateObject private var store = AlertStore.shared
    @StateObject private var info = PortfolioAssetInfoViewModel()
    @FocusState private var valueFocused: Bool
    @State private var direction: Direction = .crossesAbove
    @State private var target = ""
    @State private var percent = ""
    @State private var currency: PortfolioCurrency = .USD
    @State private var frequency: AlertFrequency = .once
    @State private var note = ""
    @State private var sendsToWebhooks = false
    @State private var webhookIDs: [UUID] = []
    @State private var webhookMessage = ""
    /// The price a percentage alert counts from; fixed when the sheet opens (or kept from the saved alert).
    @State private var reference: Decimal?
    /// The market now, in the chosen currency; refreshed while the sheet is open.
    @State private var currentPrice: Decimal?
    @State private var identical = false

    init(asset: PortfolioAsset, existing: PriceAlert? = nil) {
        self.asset = asset
        self.existing = existing
        let quote = PortfolioCurrency.alertCurrencies.contains(asset.quoteCurrency) ? asset.quoteCurrency : .USD
        _currency = State(initialValue: existing?.currency ?? quote)
        _frequency = State(initialValue: existing?.frequency ?? .once)
        _note = State(initialValue: existing?.note ?? "")
        _sendsToWebhooks = State(initialValue: !(existing?.webhookEndpointIDs.isEmpty ?? true))
        _webhookIDs = State(initialValue: existing?.webhookEndpointIDs ?? [])
        _webhookMessage = State(initialValue: existing?.webhookMessage ?? "")
        switch existing?.condition {
        case .crossesAbove(let level):
            _target = State(initialValue: level.description)
        case .crossesBelow(let level):
            _direction = State(initialValue: .crossesBelow)
            _target = State(initialValue: level.description)
        case .risesBy(let amount, let baseline, _):
            _direction = State(initialValue: .rises)
            _percent = State(initialValue: amount.description)
            _reference = State(initialValue: baseline)
        case .fallsBy(let amount, let baseline, _):
            _direction = State(initialValue: .falls)
            _percent = State(initialValue: amount.description)
            _reference = State(initialValue: baseline)
        case .unsupported, nil: break
        }
    }

    var body: some View {
        FormScrollContainer {
            form
        } footer: {
            footer
        }
        .frame(width: 480)
        .task { await populate() }
        .task { await keepPriceFresh() }
        .task { info.load([asset]) }
        .task(id: duplicateKey) { identical = await isDuplicate() }
        .onAppear { valueFocused = true }
        .onChange(of: currency) { _, value in
            Task {
                currentPrice = await store.latestPrice(for: asset, currency: value)
                reference = currentPrice
            }
        }
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 18) {
            SheetHeader(
                title: existing == nil ? "Create Price Alert" : "Edit Price Alert",
                subtitle: "\(info.subtitle(for: asset)) · \(asset.source.displayName)"
            ) {
                AlertAssetIcon(asset: asset, info: info, size: 40)
            }
            priceCard
            conditionSection
            valueSection
            frequencySection
            noteSection
            WebhookSendSection(
                isOn: $sendsToWebhooks, selection: $webhookIDs, message: $webhookMessage,
                exampleContext: webhookExampleContext)
            if identical {
                NoticeCard(
                    systemImage: "exclamationmark.triangle.fill", tint: .orange,
                    title: "An identical alert already exists",
                    detail: "Saving adds a second one that fires at the same time.")
            }
        }
    }

    /// Example values for the webhook message preview: what this alert would send for its own market.
    private var webhookExampleContext: AlertMessageContext {
        AlertMessageContext(
            ticker: asset.metadata["apiSymbol"] ?? asset.symbol, exchange: asset.source.displayName, interval: "1",
            close: currentPrice.map { WebhookPickerLogic.exampleClose($0, currency: currency) }, time: Date(),
            now: Date())
    }

    // MARK: - Sections

    private var priceCard: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Current price").font(.caption).foregroundStyle(.secondary)
                Text(currentPrice.map { currency.formatAlertPrice($0) } ?? "—")
                    .font(.title2.weight(.semibold).monospacedDigit())
                    .contentTransition(.numericText())
            }
            Spacer()
            SettingsStatusBadge(
                text: currentPrice == nil ? "No price yet" : "Live", tone: currentPrice == nil ? .warning : .good)
        }
        .padding(14)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(.separator.opacity(0.5)))
        .accessibilityElement(children: .combine)
    }

    private var conditionSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionTitle("Alert me when the price")
            IconTabBar(
                items: Direction.allCases.map {
                    IconTabBar<Direction>.Item(value: $0, title: $0.rawValue, systemImage: $0.systemImage)
                },
                selection: $direction, isCompact: true, fillsWidth: true)
            Text(direction.blurb).font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var valueSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            if direction.isPercent {
                valueField(label: "By") {
                    TextField("0", text: $percent)
                        .textFieldStyle(.plain)
                        .font(.title3.weight(.semibold).monospacedDigit())
                        .focused($valueFocused)
                    Text("%").font(.title3).foregroundStyle(.secondary)
                }
                chips(percentOptions.map { (label: "\($0)%", value: String($0), isSelected: percent == String($0)) }) {
                    percent = $0
                }
                if reference == nil {
                    hint("A fresh current price is required for percentage alerts.", tone: .orange)
                } else if let level = condition?.target {
                    hint("Fires at \(currency.formatAlertPrice(level))", tone: .secondary)
                }
            } else {
                valueField(label: "Target price") {
                    Text(currency.glyph).font(.title3).foregroundStyle(.secondary)
                    TextField("0.00", text: $target)
                        .textFieldStyle(.plain)
                        .font(.title3.weight(.semibold).monospacedDigit())
                        .focused($valueFocused)
                    currencyMenu
                }
                if currentPrice != nil {
                    chips(
                        offsetOptions.map {
                            (label: $0.label, value: $0.targetText, isSelected: target == $0.targetText)
                        }
                    ) {
                        target = $0
                    }
                }
                if let targetHint { hint(targetHint.text, tone: targetHint.tone) }
            }
        }
    }

    /// The value input with its label in front of it, like a row in a settings form. Tapping anywhere
    /// on the row, label included, focuses the field.
    private func valueField<Content: View>(label: String, @ViewBuilder _ content: () -> Content) -> some View {
        field {
            Text(label)
                .font(.subheadline.weight(.medium))
                .foregroundStyle(.secondary)
                .fixedSize()
            Divider().frame(height: 18)
            content()
        }
        .contentShape(Rectangle())
        .onTapGesture { valueFocused = true }
    }

    private var currencyMenu: some View {
        Menu {
            Picker("Currency", selection: $currency) {
                ForEach(PortfolioCurrency.alertCurrencies) { Text("\($0.rawValue) — \($0.displayName)").tag($0) }
            }
            .pickerStyle(.inline)
        } label: {
            Text(currency.rawValue).font(.callout.weight(.semibold))
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help("Currency the alert is checked in")
    }

    /// A slim row, not a section: most alerts keep the default, so it stays out of the way.
    private var frequencySection: some View {
        HStack(spacing: 12) {
            Text("How often")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Spacer(minLength: 8)
            Picker("How often", selection: $frequency) {
                Text("Once").tag(AlertFrequency.once)
                Text("Every time").tag(AlertFrequency.everyTime)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.small)
            .fixedSize()
        }
        .help(
            frequency == .once
                ? "Fires once, then waits until you turn it back on."
                : "Fires on each crossing, re-arming in between.")
    }

    private var noteSection: some View {
        field {
            Image(systemName: "text.alignleft").foregroundStyle(.secondary)
            TextField("Add a note (optional)", text: $note).textFieldStyle(.plain)
        }
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Text(summary ?? "Set a level to create the alert.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .controlSize(.large)
            Button(existing == nil ? "Create Alert" : "Save Changes") { Task { await save() } }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .controlSize(.large)
                .disabled(condition == nil)
        }
    }

    // MARK: - Pieces

    private func sectionTitle(_ text: String) -> some View {
        Text(text).font(.subheadline.weight(.semibold))
    }

    private func field<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        HStack(spacing: 8) { content() }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.6)))
    }

    private func hint(_ text: String, tone: Color) -> some View {
        Text(text).font(.caption).foregroundStyle(tone).fixedSize(horizontal: false, vertical: true)
    }

    /// Quick values as pills; the one already typed is highlighted.
    private func chips(_ items: [(label: String, value: String, isSelected: Bool)], pick: @escaping (String) -> Void)
        -> some View
    {
        HStack(spacing: 6) {
            ForEach(items, id: \.label) { item in
                Button {
                    pick(item.value)
                } label: {
                    Text(item.label)
                        .font(.caption.weight(.semibold).monospacedDigit())
                        .foregroundStyle(item.isSelected ? Color.accentColor : .secondary)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                        .background(
                            (item.isSelected ? Color.accentColor : Color.primary).opacity(
                                item.isSelected ? 0.14 : 0.06),
                            in: Capsule())
                }
                .buttonStyle(.plain)
            }
            Spacer(minLength: 0)
        }
    }

    // MARK: - Values

    private var percentOptions: [Int] { [1, 2, 5, 10, 20] }

    /// Targets a few percent away from the current price, on the side the alert watches.
    private var offsetOptions: [TargetOffset] {
        let sign = direction == .crossesBelow ? -1 : 1
        return [1, 2, 5, 10].compactMap { step in
            currentPrice.map { TargetOffset(percent: step * sign, from: $0, currency: currency) }
        }
    }

    private struct TargetOffset {
        let percent: Int
        let targetText: String

        init(percent: Int, from price: Decimal, currency: PortfolioCurrency) {
            self.percent = percent
            let level = price * (1 + Decimal(percent) / 100)
            // A plain, locale-independent number: the field is parsed with `Decimal(string:)`.
            targetText = level.formatted(
                .number.grouping(.never)
                    .precision(
                        .fractionLength(
                            PortfolioCurrency.alertFractionDigits(for: abs(level), wholeUnits: currency == .JPY))
                    )
                    .locale(Locale(identifier: "en_US_POSIX")))
        }

        var label: String { (percent > 0 ? "+" : "−") + "\(abs(percent))%" }
    }

    private var parsedTarget: Decimal? { Decimal(string: target.trimmingCharacters(in: .whitespaces)) }

    /// How the typed level sits against the market, and a warning when the alert could not fire soon.
    private var targetHint: (text: String, tone: Color)? {
        guard let level = parsedTarget, level > 0, let price = currentPrice, price > 0 else { return nil }
        let change = (level - price) / price * 100
        let signed = (change >= 0 ? "+" : "−") + abs(change).formatted(.number.precision(.fractionLength(1))) + "%"
        if direction == .crossesAbove && level <= price {
            return (
                "\(signed) from now. The price is already above this level; it fires after dropping below and crossing back up.",
                .orange
            )
        }
        if direction == .crossesBelow && level >= price {
            return (
                "\(signed) from now. The price is already below this level; it fires after rising above and crossing back down.",
                .orange
            )
        }
        return ("\(signed) from the current price", .secondary)
    }

    private var condition: AlertCondition? {
        switch direction {
        case .crossesAbove:
            if let value = parsedTarget, value > 0 { return .crossesAbove(target: value) }
            return nil
        case .crossesBelow:
            if let value = parsedTarget, value > 0 { return .crossesBelow(target: value) }
            return nil
        case .rises, .falls:
            guard let value = Decimal(string: percent.trimmingCharacters(in: .whitespaces)), value > 0, let reference
            else { return nil }
            let fraction = value / 100
            return direction == .rises
                ? .risesBy(percent: value, reference: reference, target: reference * (1 + fraction))
                : .fallsBy(percent: value, reference: reference, target: reference * (1 - fraction))
        }
    }

    /// "BTC crosses above $70,000.00 · once", shown beside the Save button.
    private var summary: String? {
        guard let condition else { return nil }
        let ticker = asset.displayTicker
        let price = { (value: Decimal) in currency.formatAlertPrice(value) }
        let what: String
        switch condition {
        case .crossesAbove(let level): what = "\(ticker) crosses above \(price(level))"
        case .crossesBelow(let level): what = "\(ticker) crosses below \(price(level))"
        case .risesBy(let percent, _, let level): what = "\(ticker) rises \(percent.description)% to \(price(level))"
        case .fallsBy(let percent, _, let level): what = "\(ticker) falls \(percent.description)% to \(price(level))"
        case .unsupported: return nil
        }
        return what + " · " + (frequency == .once ? "once" : "every time")
    }

    // MARK: - Actions

    private var candidate: PriceAlert? {
        condition.map {
            PriceAlert(
                id: existing?.id ?? UUID(), asset: asset, condition: $0, currency: currency, frequency: frequency)
        }
    }

    /// Changes whenever the rule being built changes, so the duplicate check reruns.
    private var duplicateKey: String { "\(String(describing: condition))|\(currency.rawValue)|\(frequency.rawValue)" }

    private func isDuplicate() async -> Bool {
        guard let candidate else { return false }
        return await store.isIdentical(candidate)
    }

    /// The market moves while the sheet is open; keep the price and the hints honest.
    private func keepPriceFresh() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(10))
            let price = await store.latestPrice(for: asset, currency: currency)
            if let price { currentPrice = price }
        }
    }

    private func populate() async {
        currentPrice = await store.latestPrice(for: asset, currency: currency)
        // A saved percentage alert keeps the price it was counting from.
        if reference == nil { reference = currentPrice }
    }

    private func save() async {
        guard let condition else { return }
        var alert = existing ?? PriceAlert(asset: asset, condition: condition)
        alert.condition = condition
        alert.currency = currency
        alert.frequency = frequency
        alert.note = note
        // Off means no webhooks, whatever was picked before; the choice stays in the form if it is turned back on.
        alert.webhookEndpointIDs = sendsToWebhooks ? webhookIDs : []
        let message = webhookMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        alert.webhookMessage = message.isEmpty ? nil : webhookMessage
        alert.state = .active
        store.save(alert)
        dismiss()
    }
}
