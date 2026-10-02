import SwiftUI

/// First step of a CoinMarketCap import: match each exported ticker to a market we can price,
/// resolve foreign-currency fees, then hand the converted transactions to the preview sheet.
struct PortfolioCoinMarketCapImportSheet: View {
    @ObservedObject var store: PortfolioStore
    let preview: CoinMarketCapCSVPreview
    let portfolioID: UUID
    let onReady: (PortfolioCSVPreview) -> Void
    @Environment(\.dismiss) private var dismiss
    @StateObject private var info = PortfolioAssetInfoViewModel()
    @State private var mappings: [String: PortfolioAsset] = [:]
    @State private var skippedSymbols: Set<String> = []
    @State private var mappingSymbol: String?
    @State private var resolvingSymbols: Set<String> = []
    @State private var autoMappedSymbols: Set<String> = []
    @State private var didAutoMap = false
    @State private var feeFXRates: [String: Decimal] = [:]
    @State private var feeFXErrors: [String: String] = [:]
    @State private var isResolvingFeeFX = false
    @State private var feeFXProgress = 0.0
    @State private var skippedRowIDs: Set<String> = []

    private var mappedRows: Int {
        preview.rows.filter { mappings[$0.token] != nil && !skippedRowIDs.contains($0.id) }.count
    }
    private var skippedRows: Int { preview.rows.count - mappedRows }
    private var portfolioCurrency: PortfolioCurrency {
        store.snapshot.portfolios.first { $0.id == portfolioID }?.baseCurrency ?? .USD
    }
    private var foreignFeeRows: [CoinMarketCapCSVRow] {
        preview.rows.filter { row in
            mappings[row.token] != nil && !skippedRowIDs.contains(row.id) && row.fee != 0
                && (row.feeCurrency ?? .USD) != .USD
        }
    }
    private var canContinue: Bool {
        preview.isValid && mappedRows > 0 && didAutoMap && !isResolvingFeeFX
            && !foreignFeeRows.contains { feeFXRates[$0.id] == nil && !skippedRowIDs.contains($0.id) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            PortfolioSheetHeader(
                systemImage: "square.and.arrow.down.on.square.fill", title: "Import from CoinMarketCap",
                subtitle: "\(preview.rows.count) transactions · \(preview.symbols.count) assets"
            )
            .padding(.horizontal, 24)
            .padding(.top, 24)
            .padding(.bottom, 16)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if !preview.errors.isEmpty {
                        PortfolioNoticeCard(
                            systemImage: "xmark.octagon.fill", tint: .red,
                            title: "\(preview.errors.count) CSV \(preview.errors.count == 1 ? "error requires" : "errors require") attention",
                            lines: preview.errors, maxListHeight: 100)
                    }
                    tokensSection
                    if !foreignFeeRows.isEmpty { feeSection }
                    if !skippedRowIDs.isEmpty {
                        Button(
                            "Restore \(skippedRowIDs.count) individually skipped "
                                + "\(skippedRowIDs.count == 1 ? "transaction" : "transactions")"
                        ) { skippedRowIDs.removeAll() }
                        .font(.caption)
                    }
                    if !preview.warnings.isEmpty {
                        PortfolioNoticeCard(
                            systemImage: "exclamationmark.triangle.fill", tint: .orange,
                            title: "\(preview.warnings.count) \(preview.warnings.count == 1 ? "warning" : "warnings")",
                            lines: preview.warnings, maxListHeight: 80)
                    }
                }
                .padding(.horizontal, 24)
                .padding(.bottom, 16)
            }
            Divider()
            footer
        }
        .frame(width: 700, height: 640)
        .sheet(isPresented: Binding(get: { mappingSymbol != nil }, set: { if !$0 { mappingSymbol = nil } })) {
            AddTickerSheet(title: "Map \(mappingSymbol ?? "Token")", actionLabel: "Use Asset") { result in
                guard let symbol = mappingSymbol else { return }
                mappings[symbol] = PortfolioAsset(searchResult: result)
                skippedSymbols.remove(symbol)
                autoMappedSymbols.remove(symbol)
                mappingSymbol = nil
            }
        }
        .onChange(of: mappings) { _, new in info.load(Array(new.values)) }
        .task {
            applyExistingMappings()
            async let mapping: Void = autoMapRemainingSymbols()
            async let rates: Void = resolveHistoricalFeeRates()
            _ = await (mapping, rates)
        }
    }

    // MARK: - Tokens

    private var tokensSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                Text("Match tokens to markets").font(.headline)
                Text(
                    "Only transactions for matched tickers are imported. "
                        + "Skip a ticker to leave all of its transactions out."
                )
                .font(.subheadline)
                .foregroundStyle(.secondary)
            }
            if !didAutoMap {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Finding \(portfolioCurrency.rawValue) markets — Binance, then Coinbase, CoinGecko and DEXScreener…")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            VStack(spacing: 6) {
                ForEach(preview.symbols, id: \.self) { symbol in
                    TokenRow(
                        symbol: symbol,
                        transactionCount: preview.rows.filter { $0.token == symbol }.count,
                        asset: mappings[symbol], info: info,
                        isResolving: resolvingSymbols.contains(symbol),
                        isAuto: autoMappedSymbols.contains(symbol),
                        isSkipped: skippedSymbols.contains(symbol),
                        onMap: {
                            skippedSymbols.remove(symbol)
                            autoMappedSymbols.remove(symbol)
                            mappingSymbol = symbol
                        },
                        onSkip: { skippedSymbols.insert(symbol) },
                        onReset: {
                            mappings[symbol] = nil
                            skippedSymbols.remove(symbol)
                            autoMappedSymbols.remove(symbol)
                        })
                }
            }
        }
    }

    // MARK: - Fees in other currencies

    private var feeSection: some View {
        let failed = foreignFeeRows.filter { feeFXErrors[$0.id] != nil }
        return VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "eurosign.arrow.circlepath").foregroundStyle(Color.accentColor)
                Text("Fee exchange rates").font(.headline)
            }
            if isResolvingFeeFX {
                HStack(spacing: 10) {
                    ProgressView(value: feeFXProgress).progressViewStyle(.linear)
                    Text("\(Int((feeFXProgress * 100).rounded()))%").monospacedDigit().font(.caption)
                }
                Text("Fetching historical exchange rates…").font(.caption).foregroundStyle(.secondary)
            } else if failed.isEmpty {
                Label("Historical exchange rates loaded", systemImage: "checkmark.circle.fill")
                    .font(.caption)
                    .foregroundStyle(.green)
            }
            ForEach(failed) { row in
                HStack(spacing: 10) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(verbatim: "Line \(row.line) · \(row.token) · \(row.fee) \(row.feeCurrency?.rawValue ?? "")")
                            .font(.subheadline)
                        Text(feeFXErrors[row.id] ?? "Rate unavailable").font(.caption).foregroundStyle(.red)
                    }
                    Spacer()
                    Button("Skip Item") { skippedRowIDs.insert(row.id) }.controlSize(.small)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.5)))
    }

    // MARK: - Footer

    private var footer: some View {
        HStack(spacing: 14) {
            Label("\(mappedRows) to import", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
            if skippedRows > 0 {
                Label("\(skippedRows) skipped", systemImage: "minus.circle").foregroundStyle(.secondary)
            }
            Spacer()
            Button("Cancel") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .controlSize(.large)
            Button("Continue to Preview") {
                onReady(
                    preview.transactions(
                        portfolioID: portfolioID, mappings: mappings,
                        feeFXRates: feeFXRates, skippedRowIDs: skippedRowIDs))
            }
            .keyboardShortcut(.defaultAction)
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(!canContinue)
        }
        .font(.callout)
        .padding(.horizontal, 24)
        .padding(.vertical, 14)
    }

    // MARK: - Work

    private func resolveHistoricalFeeRates() async {
        let rows = preview.rows.filter { $0.fee != 0 && ($0.feeCurrency ?? .USD) != .USD }
        guard !rows.isEmpty else { return }
        isResolvingFeeFX = true
        feeFXProgress = 0
        defer { isResolvingFeeFX = false }
        for (index, row) in rows.enumerated() {
            let currency = row.feeCurrency ?? .USD
            do {
                feeFXRates[row.id] = try await FXRateService.shared.conversion(
                    from: currency, to: .USD, on: row.timestamp
                ).rate
                feeFXErrors[row.id] = nil
            } catch {
                feeFXErrors[row.id] = error.localizedDescription
            }
            feeFXProgress = Double(index + 1) / Double(rows.count)
        }
    }

    private func applyExistingMappings() {
        for symbol in preview.symbols {
            let assets = Set(
                store.snapshot.transactions.filter { $0.asset.symbol.caseInsensitiveCompare(symbol) == .orderedSame }
                    .map(\.asset))
            if assets.count == 1 { mappings[symbol] = assets.first }
        }
    }

    private func autoMapRemainingSymbols() async {
        guard !didAutoMap else { return }
        for symbol in preview.symbols where mappings[symbol] == nil && !skippedSymbols.contains(symbol) {
            resolvingSymbols.insert(symbol)
            if let asset = await PortfolioAssetAutoMapper.resolve(symbol: symbol, baseCurrency: portfolioCurrency),
                mappings[symbol] == nil, !skippedSymbols.contains(symbol)
            {
                mappings[symbol] = asset
                autoMappedSymbols.insert(symbol)
            }
            resolvingSymbols.remove(symbol)
        }
        didAutoMap = true
    }
}

/// One exported ticker and what it maps to: icon, counts, status, and the actions that change it.
private struct TokenRow: View {
    let symbol: String
    let transactionCount: Int
    let asset: PortfolioAsset?
    @ObservedObject var info: PortfolioAssetInfoViewModel
    let isResolving: Bool
    let isAuto: Bool
    let isSkipped: Bool
    let onMap: () -> Void
    let onSkip: () -> Void
    let onReset: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            TickerIconView(
                symbol: asset?.displayTicker ?? symbol, url: asset.flatMap(info.iconURL(for:)), size: 32)
            VStack(alignment: .leading, spacing: 1) {
                Text(symbol).font(.body.weight(.semibold))
                Text("\(transactionCount) \(transactionCount == 1 ? "transaction" : "transactions")")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(width: 130, alignment: .leading)
            Image(systemName: "arrow.right").font(.caption).foregroundStyle(.tertiary)
            status
            Spacer(minLength: 8)
            actions
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.4)))
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder private var status: some View {
        if isResolving {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Searching exchanges…").font(.subheadline).foregroundStyle(.secondary)
            }
        } else if let asset {
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    Text(asset.displayTicker).font(.subheadline.weight(.semibold))
                    if isAuto { pill("Auto-matched", tint: .blue) }
                }
                Text("\(info.subtitle(for: asset)) · \(asset.source.rawValue)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        } else if isSkipped {
            VStack(alignment: .leading, spacing: 1) {
                pill("Skipped", tint: .secondary)
                Text("Won't be imported").font(.caption).foregroundStyle(.secondary)
            }
        } else {
            VStack(alignment: .leading, spacing: 1) {
                pill("Not matched", tint: .orange)
                Text("Skipped unless you match it").font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder private var actions: some View {
        if !isResolving {
            HStack(spacing: 6) {
                if asset != nil || isSkipped {
                    Button("Reset", action: onReset).buttonStyle(.borderless)
                }
                if asset == nil && !isSkipped {
                    Button("Skip", action: onSkip)
                }
                Button(asset == nil ? "Match…" : "Change…", action: onMap)
            }
            .controlSize(.small)
        }
    }

    private func pill(_ text: String, tint: Color) -> some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(tint.opacity(0.14), in: Capsule())
    }
}
