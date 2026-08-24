import SwiftUI

// MARK: - ChartSettingsSheet

struct ChartSettingsSheet: View {
    @ObservedObject var viewModel: ChartViewModel
    /// (symbol, source, displayName, pmSeries) — `displayName` is nil for crypto sources,
    /// whose symbol already reads fine on the card.
    let onUpdateTicker: (String, DataSourceType, String?, [PmSeriesConfig]?) -> Void
    let onRemove: () -> Void
    let onStyleChanged: () -> Void

    @StateObject private var searchVM = TickerSearchViewModel(logPrefix: "[ChartSettings]")
    @StateObject private var stockVM = TickerSearchViewModel(
        logPrefix: "[ChartSettings/Stocks]",
        sources: { [DataSourceFactory.shared.alpaca] }
    )
    @StateObject private var polymarketVM = PolymarketSearchViewModel(logPrefix: "[ChartSettings/Polymarket]")

    @State private var selectedTab: Tab
    @State private var searchText = ""
    @State private var polymarketText = ""
    @State private var stockText = ""
    @State private var assetType: ChartAssetType

    // Appearance state — initialized from viewModel
    @State private var bullishColor: Color
    @State private var bearishColor: Color
    @State private var decimalPlacesMode: DecimalMode
    @State private var showVolume: Bool
    @State private var showRSI: Bool
    @State private var showEMA: Bool
    @State private var emaPeriod: Int
    @State private var showBollinger: Bool
    @State private var showTrendFlips: Bool
    @State private var pineDraft: String

    @Environment(\.dismiss) private var dismiss

    enum Tab: String, CaseIterable {
        case ticker = "Ticker"
        case appearance = "Appearance"
        case indicators = "Indicators"
        case scripts = "Scripts"
    }

    enum DecimalMode: String, CaseIterable, Identifiable {
        case auto = "Auto"
        case zero = "0"
        case one = "1"
        case two = "2"
        case three = "3"
        case four = "4"
        case five = "5"
        case six = "6"
        case seven = "7"
        case eight = "8"

        var id: String { rawValue }

        var intValue: Int? {
            switch self {
            case .auto: return nil
            default: return Int(rawValue)!
            }
        }

        static func from(_ int: Int?) -> DecimalMode {
            guard let int else { return .auto }
            switch int {
            case 0: return .zero
            case 1: return .one
            case 2: return .two
            case 3: return .three
            case 4: return .four
            case 5: return .five
            case 6: return .six
            case 7: return .seven
            case 8: return .eight
            default: return .auto
            }
        }
    }

    init(viewModel: ChartViewModel,
         onUpdateTicker: @escaping (String, DataSourceType, String?, [PmSeriesConfig]?) -> Void,
         onRemove: @escaping () -> Void,
         onStyleChanged: @escaping () -> Void) {
        self.viewModel = viewModel
        self.onUpdateTicker = onUpdateTicker
        self.onRemove = onRemove
        self.onStyleChanged = onStyleChanged
        _bullishColor = State(initialValue: viewModel.bullishColor)
        _bearishColor = State(initialValue: viewModel.bearishColor)
        _decimalPlacesMode = State(initialValue: DecimalMode.from(viewModel.yAxisDecimalPlaces))
        _showVolume = State(initialValue: viewModel.showVolume)
        _showRSI = State(initialValue: viewModel.showRSI)
        _showEMA = State(initialValue: viewModel.showEMA)
        _emaPeriod = State(initialValue: viewModel.emaPeriod)
        _showBollinger = State(initialValue: viewModel.showBollinger)
        _showTrendFlips = State(initialValue: viewModel.showTrendFlips)
        _pineDraft = State(initialValue: viewModel.pineConfiguration?.draftSource ?? "//@version=6\nindicator(\"My Indicator\", overlay=true)\n\nplot(close)\n")
        // Open on the tab that matches what this chart already is.
        _selectedTab = State(initialValue: .ticker)
        _assetType = State(initialValue: viewModel.source == .polymarket ? .polymarket : (viewModel.source == .alpaca ? .stock : .crypto))
    }

    var body: some View {
        VStack(spacing: 0) {
            // Title row with native-style close button
            HStack(spacing: 0) {
                Button {
                    cancelSearches()
                    dismiss()
                } label: {
                    Circle()
                        .fill(Color(nsColor: .systemRed))
                        .frame(width: 12, height: 12)
                }
                .buttonStyle(.plain)

                Spacer()

                Text("\(viewModel.title) Settings")
                    .font(.headline)
                    .lineLimit(1)
                    .truncationMode(.tail)

                Spacer()

                // Balance the close button so the title stays centered.
                Circle().frame(width: 12, height: 12).opacity(0)
            }
            .padding(.horizontal, 16)
            .padding(.top, 16)
            .padding(.bottom, 8)

            // Tab picker
            Picker("", selection: $selectedTab) {
                ForEach(Tab.allCases, id: \.self) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 16)
            .padding(.bottom, 8)

            Divider()

            switch selectedTab {
            case .ticker:
                tickerTab
            case .appearance:
                appearanceTab
            case .indicators:
                indicatorsTab
            case .scripts:
                scriptsTab
            }

            Divider()

            // Bottom buttons
            HStack {
                Button(role: .destructive) {
                    dismiss()
                    onRemove()
                } label: {
                    Label("Remove", systemImage: "trash")
                }

                Spacer()

                Button("Save") {
                    let selected = assetType == .crypto ? searchVM.selectedResult
                        : assetType == .stock ? stockVM.selectedResult
                        : polymarketVM.selectedResult
                    if selectedTab == .ticker, let selected { apply(selected) }
                    else { cancelSearches(); dismiss() }
                }
                .keyboardShortcut(.return)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .frame(width: UI.chartSettingsSheetWidth, height: UI.chartSettingsSheetHeight)
        .onDisappear {
            cancelSearches()
        }
        .onChange(of: bullishColor) {
            viewModel.bullishColor = bullishColor
            onStyleChanged()
        }
        .onChange(of: bearishColor) {
            viewModel.bearishColor = bearishColor
            onStyleChanged()
        }
        .onChange(of: decimalPlacesMode) {
            viewModel.yAxisDecimalPlaces = decimalPlacesMode.intValue
            onStyleChanged()
        }
        .onChange(of: showVolume) {
            viewModel.showVolume = showVolume
            onStyleChanged()
        }
        .onChange(of: showRSI) {
            viewModel.showRSI = showRSI
            onStyleChanged()
        }
        .onChange(of: showEMA) {
            viewModel.showEMA = showEMA
            onStyleChanged()
        }
        .onChange(of: emaPeriod) {
            viewModel.emaPeriod = emaPeriod
            onStyleChanged()
        }
        .onChange(of: showBollinger) {
            viewModel.showBollinger = showBollinger
            onStyleChanged()
        }
        .onChange(of: showTrendFlips) {
            viewModel.showTrendFlips = showTrendFlips
            onStyleChanged()
        }
        .onChange(of: pineDraft) { _, source in
            viewModel.updatePineDraft(source)
            onStyleChanged()
        }
    }

    // MARK: - Ticker Tab

    private var tickerTab: some View {
        VStack(spacing: 12) {
            Picker("Chart type", selection: $assetType) {
                ForEach(ChartAssetType.allCases) { type in Text(type.rawValue).tag(type) }
            }
            .padding(.horizontal, 16)

            currentChartRow

            switch assetType {
            case .crypto: cryptoTickerSearch
            case .stock: stockTickerSearch
            case .polymarket: polymarketTab
            }
        }
        .padding(.top, 8)
    }

    private var cryptoTickerSearch: some View {
        VStack(spacing: 12) {
            SearchFieldRow(
                placeholder: "New ticker symbol (e.g. BTC or PEPE)",
                text: $searchText,
                isSearching: searchVM.isSearching,
                onChange: { searchVM.scheduleSearch(query: $0) },
                onSubmit: {
                    if let first = searchVM.firstAvailableResult {
                        searchVM.selectedResult = first
                    }
                }
            )
            .padding(.horizontal, 16)

            // Search results
            if !searchVM.searchResults.isEmpty {
                List {
                    ForEach(searchVM.orderedSources, id: \.self) { source in
                        if let results = searchVM.searchResults[source], !results.isEmpty {
                            Section {
                                ForEach(results) { result in
                                    SearchResultRow(
                                        result: result,
                                        isSelected: searchVM.selectedResult == result,
                                        onSelect: { searchVM.selectedResult = result }
                                    )
                                }
                            } header: {
                                Label(source.displayName, systemImage: source.icon)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .listStyle(.inset)
                .frame(maxHeight: UI.chartSettingsResultsMaxHeight)
            }

            // Selected result
            if let selected = searchVM.selectedResult {
                SelectedResultBanner(prefix: "New", result: selected)
                    .padding(.horizontal, 16)

                Button("Change Ticker") {
                    apply(selected)
                }
                .buttonStyle(.borderedProminent)
            }

            Spacer()
        }
    }

    private var stockTickerSearch: some View {
        VStack(spacing: 12) {
            SearchFieldRow(
                placeholder: "US stock symbol or company name",
                text: $stockText,
                isSearching: stockVM.isSearching,
                onChange: { stockVM.scheduleSearch(query: $0) },
                onSubmit: { stockVM.selectedResult = stockVM.firstAvailableResult }
            )
            .padding(.horizontal, 16)

            if !AlpacaCredentialsStore.isConfigured {
                Text("Configure Alpaca in Settings before changing this chart to a stock.")
                    .font(.caption).foregroundStyle(.orange).padding(.horizontal, 16)
            }

            if let results = stockVM.searchResults[.alpaca], !results.isEmpty {
                List(results) { result in
                    SearchResultRow(result: result, isSelected: stockVM.selectedResult == result) {
                        stockVM.selectedResult = result
                    }
                }
                .listStyle(.inset).frame(maxHeight: UI.chartSettingsResultsMaxHeight)
            }
            if let selected = stockVM.selectedResult {
                SelectedResultBanner(prefix: "New", result: selected).padding(.horizontal, 16)
                Button("Change Ticker") { apply(selected) }.buttonStyle(.borderedProminent)
            }
            Spacer()
        }
    }

    // MARK: - Polymarket Tab

    private var polymarketTab: some View {
        VStack(spacing: 12) {
            PolymarketSearchPane(
                searchVM: polymarketVM,
                searchText: $polymarketText,
                resultsMinHeight: UI.chartSettingsResultsMinHeight,
                resultsMaxHeight: UI.chartSettingsResultsMaxHeight
            )
            .padding(.horizontal, 16)

            Spacer()
        }
        .padding(.top, 8)
    }

    /// What this chart currently tracks — shown above both search panes.
    private var currentChartRow: some View {
        HStack {
            Text("Current:")
                .font(.caption)
                .foregroundStyle(.secondary)
            Label(viewModel.title, systemImage: viewModel.source.icon)
                .font(.body.weight(.medium))
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer()
        }
        .padding(.horizontal, 16)
    }

    // MARK: - Apply

    private func cancelSearches() {
        searchVM.cancelSearch()
        stockVM.cancelSearch()
        polymarketVM.cancelSearch()
    }

    private func apply(_ selected: TickerSearchResult) {
        // The search payload already carried the market artwork; seed the resolver so
        // the card repaints without another round trip.
        if selected.source == .polymarket {
            let ticker = selected.fullSymbol
            let url = selected.imageURL
            Task { await IconResolver.shared.remember(ticker: ticker, source: .polymarket, url: url) }
        }

        cancelSearches()
        dismiss()

        let displayName: String? = {
            guard selected.source == .polymarket else { return nil }
            if let series = selected.pmSeries, series.count > 1 {
                return selected.eventTitle ?? selected.symbol
            }
            return selected.symbol
        }()

        onUpdateTicker(selected.fullSymbol, selected.source, displayName, selected.pmSeries)
    }

    // MARK: - Appearance Tab

    private var appearanceTab: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 24) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(viewModel.usesLineChart ? "Line Up" : "Bullish Candle")
                        .font(.subheadline.weight(.medium))
                    ColorPicker("", selection: $bullishColor)
                        .labelsHidden()
                }

                VStack(alignment: .leading, spacing: 8) {
                    Text(viewModel.usesLineChart ? "Line Down" : "Bearish Candle")
                        .font(.subheadline.weight(.medium))
                    ColorPicker("", selection: $bearishColor)
                        .labelsHidden()
                }
            }

            Button("Reset Default Colors") {
                bullishColor = .green
                bearishColor = .red
            }
            .buttonStyle(.borderless)
            .font(.caption)
            .foregroundStyle(.secondary)

            Divider()

            VStack(alignment: .leading, spacing: 8) {
                Text("Y-Axis Decimal Places")
                    .font(.subheadline.weight(.medium))

                Picker("Decimals", selection: $decimalPlacesMode) {
                    ForEach(DecimalMode.allCases) { mode in
                        Text(mode.rawValue).tag(mode)
                    }
                }
                .pickerStyle(.menu)
                .frame(minWidth: 100)

                Text(decimalHint)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()
        }
        .padding(16)
    }

    // MARK: - Indicators Tab

    private var indicatorsTab: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                // Line sources report one price per timestamp and no turnover at all.
                if !viewModel.usesLineChart {
                    indicatorRow(hint: volumeHint) {
                        Toggle("Volume Bars", isOn: $showVolume)
                            .toggleStyle(.switch)
                            // Left visible rather than hidden: a greyed-out switch with a
                            // reason reads better than a setting that silently does nothing.
                            .disabled(!viewModel.source.providesVolume)
                    }
                }

                // Every source shares KlineData. Line sources carry flat OHLC points,
                // whose point-to-point gaps still provide Supertrend's true range.
                indicatorRow(hint: rsiHint) {
                    Toggle("RSI (\(RSI.period))", isOn: $showRSI)
                        .toggleStyle(.switch)
                }

                indicatorRow(hint: emaHint) {
                    HStack {
                        Toggle("EMA", isOn: $showEMA)
                            .toggleStyle(.switch)

                        Picker("Period", selection: $emaPeriod) {
                            ForEach(Indicator.emaPeriods, id: \.self) { period in
                                Text("\(period)").tag(period)
                            }
                        }
                        .pickerStyle(.menu)
                        .labelsHidden()
                        .frame(width: 80)
                        .disabled(!showEMA)
                    }
                }

                indicatorRow(hint: bollingerHint) {
                    Toggle("Bollinger Bands", isOn: $showBollinger)
                        .toggleStyle(.switch)
                }

                indicatorRow(hint: trendFlipsHint) {
                    Toggle("Trend Flips (Supertrend)", isOn: $showTrendFlips)
                        .toggleStyle(.switch)
                }

                Spacer(minLength: 0)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var scriptsTab: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextEditor(text: $pineDraft)
                .font(.system(.body, design: .monospaced))
                .scrollContentBackground(.hidden)
                .padding(6)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
                .frame(minHeight: 220)

            HStack {
                Text(viewModel.pineStatus).font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button("Apply") {
                    viewModel.updatePineDraft(pineDraft)
                    if viewModel.applyPineDraft() { onStyleChanged() }
                }.buttonStyle(.borderedProminent)
            }

            if !viewModel.pineDiagnostics.isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(viewModel.pineDiagnostics) { diagnostic in
                            Text("\(diagnostic.code) · \(diagnostic.range.start.line):\(diagnostic.range.start.column)  \(diagnostic.message)")
                                .font(.caption.monospaced()).foregroundStyle(diagnostic.severity == .error ? .red : .orange)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }.frame(maxHeight: 90)
            }

            if let source=viewModel.pineConfiguration?.appliedSource {
                let schema=PineCompiler.compile(source:source).inputSchema
                ForEach(schema.inputs) { input in pineInput(input) }
            }
        }.padding(16)
    }

    @ViewBuilder private func pineInput(_ input:PineInputDefinition)->some View {
        let current=viewModel.pineConfiguration?.inputs[input.id] ?? input.defaultValue
        switch (input.type,current) {
        case (.bool,.bool(let value)):
            Toggle(input.title ?? input.id,isOn:Binding(get:{value},set:{viewModel.setPineInput(.bool($0),id:input.id);onStyleChanged()}))
        case (.int,.int(let value)):
            Stepper("\(input.title ?? input.id): \(value)",value:Binding(get:{value},set:{viewModel.setPineInput(.int($0),id:input.id);onStyleChanged()}),in:Int(input.minValue ?? 1)...Int(input.maxValue ?? 10_000),step:Int(input.step ?? 1))
        case (.float,.float(let value)):
            HStack { Text(input.title ?? input.id); TextField("",value:Binding(get:{value},set:{viewModel.setPineInput(.float($0),id:input.id);onStyleChanged()}),format:.number).frame(width:100) }
        case (.string,.string(let value)):
            HStack { Text(input.title ?? input.id); TextField("",text:Binding(get:{value},set:{viewModel.setPineInput(.string($0),id:input.id);onStyleChanged()})) }
        default: EmptyView()
        }
    }

    /// A toggle (or toggle plus its options) above the line of explanation that
    /// belongs to it.
    private func indicatorRow<Control: View>(
        hint: String,
        @ViewBuilder control: () -> Control
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            control()
            Text(hint)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var volumeHint: String {
        guard viewModel.source.providesVolume else {
            return "\(viewModel.source.displayName) doesn't report per-candle volume, so bars aren't available for this chart."
        }
        return "Turnover per candle, drawn along the bottom of the chart."
    }

    /// Flags the warm-up explicitly: RSI has no value for its first `period` candles,
    /// so a short range draws a stub of a line or none at all.
    private var rsiHint: String {
        let loaded = viewModel.klineData.count
        if loaded > 0 && loaded <= RSI.period {
            return "Only \(loaded) candles loaded — RSI needs more than \(RSI.period). Zoom out or pick a longer range."
        }
        return "Momentum from 0–100 across the bottom, with guides at \(Int(RSI.oversold)) and \(Int(RSI.overbought))."
    }

    private var emaHint: String {
        "Exponential moving average over the closes, drawn on the price scale."
    }

    private var bollingerHint: String {
        "\(Indicator.bollingerPeriod)-period average with bands \(Int(Indicator.bollingerMultiplier)) standard deviations either side — wide when volatile, tight when calm."
    }

    private var trendFlipsHint: String {
        "Bullish and bearish markers from \(Indicator.supertrendPeriod)-period ATR × \(Int(Indicator.supertrendMultiplier)). Signals appear after the candle closes and can whipsaw in sideways markets."
    }

    private var decimalHint: String {
        switch decimalPlacesMode {
        case .auto:
            "Automatically chooses precision based on price range."
        default:
            "Shows \(decimalPlacesMode.rawValue) decimal place\(decimalPlacesMode.rawValue == "1" ? "" : "s") on the Y-axis."
        }
    }
}

#Preview {
    let vm = ChartViewModel(ticker: "BTC")
    ChartSettingsSheet(
        viewModel: vm,
        onUpdateTicker: { _, _, _, _ in },
        onRemove: {},
        onStyleChanged: {}
    )
}
