import SwiftUI

struct PriceAlertEditor: View {
    let asset: PortfolioAsset
    var existing: PriceAlert?
    @Environment(\.dismiss) private var dismiss
    @StateObject private var store = AlertStore.shared
    @State private var direction = 0
    @State private var target = ""
    @State private var percent = ""
    @State private var currency: PortfolioCurrency = .USD
    @State private var frequency: AlertFrequency = .once
    @State private var note = ""
    @State private var reference: Decimal?
    @State private var identical = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(existing == nil ? "Create Price Alert" : "Edit Price Alert").font(.title2.bold())
            Text("\(asset.name) · \(asset.source.displayName)").foregroundStyle(.secondary)
            if let quote = store.latestQuotes[asset.key] {
                Text("Current: \(quote.price.formatted(.currency(code: quote.currency.rawValue)))")
                    .monospacedDigit()
            }
            Picker("Condition", selection: $direction) {
                Text("Crosses Above").tag(0); Text("Crosses Below").tag(1)
                Text("Rises By %").tag(2); Text("Falls By %").tag(3)
            }
            if direction < 2 {
                TextField("Target price", text: $target).textFieldStyle(.roundedBorder)
            } else {
                TextField("Percentage", text: $percent).textFieldStyle(.roundedBorder)
                HStack { suggestion(5); suggestion(10) }
                if reference == nil { Text("A fresh current price is required for percentage alerts.").font(.caption).foregroundStyle(.orange) }
            }
            Picker("Currency", selection: $currency) { ForEach(PortfolioCurrency.alertCurrencies) { Text($0.rawValue).tag($0) } }
            Picker("Frequency", selection: $frequency) { ForEach(AlertFrequency.allCases) { Text($0.rawValue).tag($0) } }.pickerStyle(.segmented)
            TextField("Note (optional)", text: $note).textFieldStyle(.roundedBorder)
            if identical { Label("An identical alert already exists.", systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange) }
            HStack { Spacer(); Button("Cancel") { dismiss() }; Button("Save") { Task { await save() } }.buttonStyle(.borderedProminent).disabled(condition == nil) }
        }
        .padding(22).frame(width: 430)
        .task { await populate() }
        .onChange(of: currency) { _, value in Task { reference = await store.latestPrice(for: asset, currency: value) } }
    }

    private func suggestion(_ value: Int) -> some View { Button("\(value)%") { percent = String(value) } }
    private var condition: AlertCondition? {
        if direction == 0, let value = Decimal(string: target), value > 0 { return .crossesAbove(target: value) }
        if direction == 1, let value = Decimal(string: target), value > 0 { return .crossesBelow(target: value) }
        guard let value = Decimal(string: percent), value > 0, let reference else { return nil }
        let fraction = value / 100
        return direction == 2 ? .risesBy(percent: value, reference: reference, target: reference * (1 + fraction)) : .fallsBy(percent: value, reference: reference, target: reference * (1 - fraction))
    }
    private func populate() async {
        reference = await store.latestPrice(for: asset, currency: currency)
        guard let existing else { return }
        currency = existing.currency; frequency = existing.frequency; note = existing.note
        switch existing.condition {
        case .crossesAbove(let v): direction = 0; target = v.description
        case .crossesBelow(let v): direction = 1; target = v.description
        case .risesBy(let p, let r, _): direction = 2; percent = p.description; reference = r
        case .fallsBy(let p, let r, _): direction = 3; percent = p.description; reference = r
        case .unsupported: break
        }
    }
    private func save() async {
        guard let condition else { return }
        var alert = existing ?? PriceAlert(asset: asset, condition: condition)
        alert.condition = condition; alert.currency = currency; alert.frequency = frequency; alert.note = note; alert.state = .active
        if await store.isIdentical(alert) { identical = true }
        await store.save(alert); dismiss()
    }
}
