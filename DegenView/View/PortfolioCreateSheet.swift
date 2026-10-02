import SwiftUI

/// Name a new portfolio and pick the currency its transactions are valued in.
struct PortfolioCreateSheet: View {
    @ObservedObject var store: PortfolioStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var currency = PortfolioCurrency.USD
    @FocusState private var nameFocused: Bool

    private var isValid: Bool { PortfolioNameCheck.isValid(name) }
    private var isDuplicate: Bool {
        PortfolioNameCheck.isDuplicate(name, among: store.snapshot.portfolios.map(\.name))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 22) {
                header
                nameField
                currencyPicker
            }
            .padding(24)
            Divider()
            footer
        }
        .frame(width: 460)
        .onAppear { nameFocused = true }
    }

    // MARK: - Header

    private var header: some View {
        PortfolioSheetHeader(
            systemImage: "briefcase.fill", title: "New Portfolio",
            subtitle: "Group transactions and track them together.")
    }

    // MARK: - Name

    private var nameField: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Name").font(.subheadline.weight(.medium))
            TextField("e.g. Long-term holds", text: $name)
                .textFieldStyle(.plain)
                .focused($nameFocused)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .strokeBorder(nameFocused ? Color.accentColor.opacity(0.7) : Color.primary.opacity(0.12))
                )
                .onChange(of: name) { _, value in
                    if value.count > PortfolioNameCheck.maxLength {
                        name = String(value.prefix(PortfolioNameCheck.maxLength))
                    }
                }
                .onSubmit(create)
            if isDuplicate {
                Label("A portfolio with this name already exists.", systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundStyle(.orange)
            }
        }
    }

    // MARK: - Currency

    private var currencyPicker: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Base currency").font(.subheadline.weight(.medium))
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: 3), spacing: 10) {
                ForEach(PortfolioCurrency.allCases) { option in
                    CurrencyTile(currency: option, isSelected: option == currency) { currency = option }
                }
            }
            Text(
                "Transactions in this portfolio are valued in \(currency.rawValue). "
                    + "You can still view it in any currency later."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: - Footer

    private var footer: some View {
        HStack {
            Spacer()
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .controlSize(.large)
            Button("Create Portfolio", action: create)
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(!isValid)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }

    private func create() {
        guard isValid else { return }
        store.create(name: PortfolioNameCheck.normalized(name), currency: currency)
        dismiss()
    }
}

/// One selectable currency: glyph, code and name.
private struct CurrencyTile: View {
    let currency: PortfolioCurrency
    let isSelected: Bool
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Text(currency.glyph)
                    .font(.system(size: 14, weight: .semibold, design: .rounded))
                    .foregroundStyle(isSelected ? Color.accentColor : .secondary)
                    .frame(width: 30, height: 30)
                    .background(
                        (isSelected ? Color.accentColor : Color.secondary).opacity(0.15), in: Circle())
                VStack(alignment: .leading, spacing: 1) {
                    Text(currency.rawValue).font(.subheadline.weight(.semibold))
                    Text(currency.displayName)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
            .background(background, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(isSelected ? Color.accentColor : Color.primary.opacity(0.12), lineWidth: isSelected ? 1.5 : 1)
            )
            .overlay(alignment: .topTrailing) {
                if isSelected {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 13))
                        .foregroundStyle(Color.accentColor)
                        .background(Circle().fill(.background).padding(2))
                        .offset(x: 5, y: -5)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .accessibilityLabel("\(currency.rawValue), \(currency.displayName)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var background: Color {
        if isSelected { return Color.accentColor.opacity(0.1) }
        return isHovered ? Color.primary.opacity(0.07) : Color.primary.opacity(0.03)
    }
}
