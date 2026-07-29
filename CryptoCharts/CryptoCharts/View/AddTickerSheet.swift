import SwiftUI

enum ValidationState {
    case idle
    case checking
    case valid
    case invalid(String) // error message
}

struct AddTickerSheet: View {
    @ObservedObject var contentViewModel: ContentViewModel

    @State private var inputText = ""
    @State private var validationState: ValidationState = .idle
    @State private var addError: String?

    @Environment(\.dismiss) private var dismiss

    private let api = BinanceAPIService()

    private let suggestions = ["BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "DOT", "LINK"]

    var body: some View {
        VStack(spacing: 20) {
            // Header
            Text("Add Ticker")
                .font(.headline)

            // Input field with validation indicator
            HStack(spacing: 8) {
                TextField("Ticker symbol (e.g. BTC or ETHUSDT)", text: $inputText)
                    .textFieldStyle(.roundedBorder)
                    .font(.body)
                    .onChange(of: inputText) {
                        validateInput()
                    }

                validationIndicator
            }

            // Suggestions
            VStack(alignment: .leading, spacing: 8) {
                Text("Suggestions")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                LazyVGrid(columns: Array(repeating: .init(.flexible()), count: 5), spacing: 8) {
                    ForEach(suggestions, id: \.self) { ticker in
                        Button(ticker) {
                            inputText = ticker
                            validateInput()
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                    }
                }
            }

            // Error from add attempt
            if let error = addError {
                HStack {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            // Action buttons
            HStack(spacing: 12) {
                Button("Cancel") {
                    dismiss()
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.escape)

                Button("Add") {
                    addTicker()
                }
                .buttonStyle(.borderedProminent)
                .disabled(!canAdd)
                .keyboardShortcut(.return)
            }
            .padding(.top, 8)
        }
        .padding(24)
        .frame(width: 400)
    }

    // MARK: - Validation Indicator

    @ViewBuilder
    private var validationIndicator: some View {
        switch validationState {
        case .idle:
            EmptyView()
        case .checking:
            ProgressView()
                .scaleEffect(0.7)
                .frame(width: 20, height: 20)
        case .valid:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .invalid:
            Image(systemName: "xmark.circle.fill")
                .foregroundStyle(.red)
        }
    }

    private var canAdd: Bool {
        if case .valid = validationState { return true }
        return false
    }

    // MARK: - Validation

    private func validateInput() {
        let text = inputText.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            validationState = .idle
            return
        }

        validationState = .checking

        let pair = text.uppercased().hasSuffix("USDT") ? text.uppercased() : "\(text.uppercased())USDT"

        Task { @MainActor in
            // Debounce: wait 300ms, but only act if input hasn't changed
            let capturedText = inputText

            try? await Task.sleep(nanoseconds: 300_000_000)

            guard inputText == capturedText else { return }

            do {
                let valid = try await api.validateSymbol(pair)
                validationState = valid ? .valid : .invalid("\"\(pair)\" not found on Binance")
            } catch {
                validationState = .invalid(error.localizedDescription)
            }
        }
    }

    // MARK: - Add

    private func addTicker() {
        let text = inputText.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return }

        Task { @MainActor in
            do {
                try await contentViewModel.addTicker(text)
                dismiss()
            } catch {
                addError = error.localizedDescription
            }
        }
    }
}

#Preview {
    AddTickerSheet(contentViewModel: ContentViewModel())
}
