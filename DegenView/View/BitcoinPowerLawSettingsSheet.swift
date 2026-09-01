import SwiftUI

struct BitcoinPowerLawSettingsSheet: View {
    let initial: BitcoinPowerLawConfig
    let onApply: (BitcoinPowerLawConfig) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var intercept: String
    @State private var exponent: String
    @State private var lowerMultiplier: String
    @State private var upperMultiplier: String

    init(initial: BitcoinPowerLawConfig, onApply: @escaping (BitcoinPowerLawConfig) -> Void) {
        self.initial = initial
        self.onApply = onApply
        _intercept = State(initialValue: String(initial.intercept))
        _exponent = State(initialValue: String(initial.exponent))
        _lowerMultiplier = State(initialValue: String(initial.lowerMultiplier))
        _upperMultiplier = State(initialValue: String(initial.upperMultiplier))
    }

    private var candidate: BitcoinPowerLawConfig? {
        guard let intercept = Double(intercept), let exponent = Double(exponent),
            let lower = Double(lowerMultiplier), let upper = Double(upperMultiplier)
        else { return nil }
        let value = BitcoinPowerLawConfig(
            intercept: intercept, exponent: exponent,
            lowerMultiplier: lower, upperMultiplier: upper)
        return value.isValid ? value : nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Bitcoin Power Law Settings").font(.headline)
            Form {
                TextField("Log₁₀ intercept", text: $intercept)
                TextField("Exponent", text: $exponent)
                TextField("Lower multiplier", text: $lowerMultiplier)
                TextField("Upper multiplier", text: $upperMultiplier)
            }
            Text("Exponent and corridor multipliers must be positive finite numbers.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Reset Defaults") { setFields(.default) }
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                Button("Apply") {
                    guard let candidate else { return }
                    onApply(candidate)
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .disabled(candidate == nil)
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 430)
    }

    private func setFields(_ config: BitcoinPowerLawConfig) {
        intercept = String(config.intercept)
        exponent = String(config.exponent)
        lowerMultiplier = String(config.lowerMultiplier)
        upperMultiplier = String(config.upperMultiplier)
    }
}
