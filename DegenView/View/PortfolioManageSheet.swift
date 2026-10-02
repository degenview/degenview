import SwiftUI

/// Rename, reorder, duplicate and delete portfolios.
struct PortfolioManageSheet: View {
    @ObservedObject var store: PortfolioStore
    @Environment(\.dismiss) private var dismiss
    @State private var deleting: Portfolio?
    @State private var showCreate = false

    private var portfolios: [Portfolio] { store.snapshot.portfolios }

    private func stats(for portfolio: Portfolio) -> (transactions: Int, assets: Int) {
        let transactions = store.snapshot.transactions.filter { $0.portfolioID == portfolio.id }
        return (transactions.count, PortfolioAccountingEngine.uniqueAssets(in: transactions).count)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
                .padding(.horizontal, 24)
                .padding(.top, 24)
                .padding(.bottom, 16)
            if portfolios.isEmpty {
                ContentUnavailableView(
                    "No Portfolios", systemImage: "briefcase",
                    description: Text("Create a portfolio to start tracking investments.")
                )
                .frame(maxHeight: .infinity)
            } else {
                list
            }
            Divider()
            footer
        }
        .frame(width: 560, height: 520)
        .sheet(isPresented: $showCreate) { PortfolioCreateSheet(store: store) }
        .confirmationDialog(
            "Delete \(deleting?.name ?? "portfolio")?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            presenting: deleting
        ) { portfolio in
            Button("Delete Portfolio and Transactions", role: .destructive) {
                store.delete(portfolio.id)
                deleting = nil
            }
            Button("Cancel", role: .cancel) { deleting = nil }
        } message: { portfolio in
            let count = stats(for: portfolio).transactions
            Text(
                "This permanently deletes the portfolio and its \(count) "
                    + "\(count == 1 ? "transaction" : "transactions"). It can't be undone.")
        }
    }

    // MARK: - Header

    private var header: some View {
        PortfolioSheetHeader(
            systemImage: "square.stack.3d.up.fill", title: "Manage Portfolios",
            subtitle: (portfolios.count == 1 ? "1 portfolio" : "\(portfolios.count) portfolios")
                + " · Drag to reorder, click a name to rename.")
    }

    // MARK: - List

    private var list: some View {
        List {
            ForEach(portfolios) { portfolio in
                let summary = stats(for: portfolio)
                ManageRow(
                    portfolio: portfolio, transactions: summary.transactions, assets: summary.assets,
                    isCurrent: store.snapshot.selectedPortfolioID == portfolio.id,
                    onRename: { name in
                        var copy = portfolio
                        copy.name = name
                        store.update(copy)
                    },
                    onDuplicate: { store.duplicate(portfolio.id) },
                    onDelete: { deleting = portfolio }
                )
                .listRowSeparator(.hidden)
                .listRowInsets(EdgeInsets(top: 3, leading: 20, bottom: 3, trailing: 20))
                .listRowBackground(Color.clear)
            }
            .onMove { store.reorder(from: $0, to: $1) }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    // MARK: - Footer

    private var footer: some View {
        HStack {
            Button {
                showCreate = true
            } label: {
                Label("New Portfolio", systemImage: "plus")
            }
            .controlSize(.large)
            Spacer()
            Button("Done") { dismiss() }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }
}

/// One portfolio: handle, currency badge, editable name, a summary line and its actions.
private struct ManageRow: View {
    let portfolio: Portfolio
    let transactions: Int
    let assets: Int
    let isCurrent: Bool
    let onRename: (String) -> Void
    let onDuplicate: () -> Void
    let onDelete: () -> Void

    @State private var name: String
    @State private var isHovered = false
    @FocusState private var focused: Bool

    init(
        portfolio: Portfolio, transactions: Int, assets: Int, isCurrent: Bool,
        onRename: @escaping (String) -> Void, onDuplicate: @escaping () -> Void, onDelete: @escaping () -> Void
    ) {
        self.portfolio = portfolio
        self.transactions = transactions
        self.assets = assets
        self.isCurrent = isCurrent
        self.onRename = onRename
        self.onDuplicate = onDuplicate
        self.onDelete = onDelete
        _name = State(initialValue: portfolio.name)
    }

    private var summary: String {
        let txText = "\(transactions) \(transactions == 1 ? "transaction" : "transactions")"
        let assetText = "\(assets) \(assets == 1 ? "asset" : "assets")"
        return "\(portfolio.baseCurrency.rawValue) · \(txText) · \(assetText)"
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "line.3.horizontal")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.tertiary)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                TextField("Name", text: $name)
                    .textFieldStyle(.plain)
                    .font(.body.weight(.semibold))
                    .focused($focused)
                    .onSubmit(commit)
                    .onChange(of: focused) { _, isFocused in if !isFocused { commit() } }
                    .onChange(of: name) { _, value in
                        if value.count > PortfolioNameCheck.maxLength {
                            name = String(value.prefix(PortfolioNameCheck.maxLength))
                        }
                    }
                Text(summary).font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            // Beside the actions, centered on the whole card rather than on the name line.
            if isCurrent { badge("Viewing", tint: .accentColor) }
            if portfolio.isArchived { badge("Archived", tint: .secondary) }
            actionButton("plus.square.on.square", help: "Duplicate, including its transactions", action: onDuplicate)
            actionButton("trash", help: "Delete…", tint: .red, action: onDelete)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(rowFill, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .strokeBorder(focused ? Color.accentColor.opacity(0.6) : Color.primary.opacity(0.1))
        )
        .onHover { isHovered = $0 }
        .onChange(of: portfolio.name) { _, value in if !focused { name = value } }
        .accessibilityElement(children: .contain)
    }

    private var rowFill: Color {
        if focused { return Color.accentColor.opacity(0.08) }
        return isHovered ? Color.primary.opacity(0.06) : Color.primary.opacity(0.03)
    }

    /// Saves a changed, non-blank name; a blank one is put back.
    private func commit() {
        let cleaned = PortfolioNameCheck.normalized(name)
        guard !cleaned.isEmpty else {
            name = portfolio.name
            return
        }
        name = cleaned
        if cleaned != portfolio.name { onRename(cleaned) }
    }

    private func badge(_ text: String, tint: Color) -> some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(tint.opacity(0.14), in: Capsule())
    }

    private func actionButton(
        _ systemImage: String, help: String, tint: Color = .secondary, action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(isHovered ? tint : Color.secondary.opacity(0.7))
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
        .accessibilityLabel("\(help.components(separatedBy: ",")[0]) \(portfolio.name)")
    }
}
