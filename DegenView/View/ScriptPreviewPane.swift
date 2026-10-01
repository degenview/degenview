import SwiftUI

/// The Script Manager's live chart: the script being edited, running on a real market.
///
/// A header to pick the market, timeframe and zoom; the chart (with a separate pane for
/// `overlay=false` scripts); and the inputs/report/problems drawer beneath it.
struct ScriptPreviewPane: View {
    @ObservedObject var preview: ScriptPreviewViewModel
    @ObservedObject var chart: ChartViewModel
    @ObservedObject var editor: ScriptEditorViewModel

    @AppStorage("scriptManager.drawerOpen") private var drawerOpen = true
    @AppStorage("scriptManager.drawerTab") private var drawerTab: ScriptPreviewDrawer.Tab = .inputs
    @AppStorage("scriptManager.drawerHeight") private var drawerHeight = 0.0

    @State private var showMarketPicker = false
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.openSettings) private var openSettings

    init(preview: ScriptPreviewViewModel, editor: ScriptEditorViewModel) {
        self.preview = preview
        self.chart = preview.chart
        self.editor = editor
    }

    private var editorHasErrors: Bool { editor.status == .error }

    private var drawer: ScriptPreviewDrawer {
        ScriptPreviewDrawer(
            preview: preview, chart: chart, editorDiagnostics: editor.diagnostics,
            editorHasErrors: editorHasErrors, tab: drawerTab)
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            SplitContainer(
                axis: .vertical, secondaryFirst: false, secondaryLength: $drawerHeight,
                isSecondaryVisible: drawerOpen, minPrimary: 140, minSecondary: 90, defaultFraction: 0.38
            ) {
                chartArea
            } secondary: {
                drawer
            }
            ScriptPreviewDrawerBar(
                tab: $drawerTab, isOpen: $drawerOpen, problemCount: drawer.diagnostics.count,
                hasErrors: drawer.diagnostics.contains { $0.severity == .error }, status: statusText)
        }
        .background(WindowAccessor { preview.attach(to: $0) })
        .onChange(of: colorScheme, initial: true) { _, scheme in
            chart.setPineTheme(scheme == .dark ? .dark : .light)
        }
        // CoinGecko tickers are coin ids; this finds the symbol the title and icon use.
        .task(id: chart.iconKey) { await chart.resolveCoinSymbol() }
    }

    private var statusText: String {
        if let reason = preview.unsupportedReason { return reason }
        return chart.pineStatus
    }

    // MARK: - Header

    private var header: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) {
                marketButton
                Spacer(minLength: 8)
                timeframePicker.frame(width: 230)
            }
            VStack(alignment: .leading, spacing: 8) {
                marketButton
                timeframePicker
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }

    private var marketButton: some View {
        Button {
            showMarketPicker = true
        } label: {
            HStack(spacing: 8) {
                ChartIconView(viewModel: chart, size: 24)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        Text(chart.title)
                            .font(.subheadline.weight(.semibold))
                            .lineLimit(1)
                        Image(systemName: "chevron.up.chevron.down")
                            .font(.system(size: 8, weight: .bold))
                            .foregroundStyle(.secondary)
                    }
                    priceLine
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Choose the market to test on")
        .popover(isPresented: $showMarketPicker, arrowEdge: .bottom) {
            ScriptPreviewMarketPicker(current: preview.market, recents: preview.recents) {
                preview.selectMarket($0)
            }
        }
    }

    @ViewBuilder private var priceLine: some View {
        if let price = chart.displayedPrice {
            HStack(spacing: 6) {
                Text(PriceFormatter.headline(price, scale: chart.priceScale))
                    .foregroundStyle(.secondary)
                if let change = chart.priceChangePercent {
                    Text("\(change >= 0 ? "+" : "−")\(abs(change).formatted(.number.precision(.fractionLength(2))))%")
                        .foregroundStyle(chart.priceChangeIsPositive ? .green : .red)
                }
            }
            .font(.caption.monospacedDigit())
        } else {
            Text(preview.market.source.displayName)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var timeframePicker: some View {
        Picker(
            "Timeframe",
            selection: Binding(get: { preview.timeRange }, set: { preview.setTimeRange($0) })
        ) {
            ForEach(TimeRange.allCases) { range in Text(range.rawValue).tag(range) }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .padding(.trailing, 8)
    }

    // MARK: - Chart

    private var chartArea: some View {
        GeometryReader { geometry in
            let paneHeight = chart.pinePaneHeight(forChartHeight: geometry.size.height)
            VStack(spacing: 0) {
                CandleChartView(
                    candles: chart.visibleKlines,
                    chartHeight: geometry.size.height - paneHeight,
                    bullishColor: chart.bullishColor,
                    bearishColor: chart.bearishColor,
                    yAxisDecimalPlaces: chart.yAxisDecimalPlaces,
                    yZoom: chart.yZoom,
                    showVolume: chart.showVolume,
                    indicators: chart.indicators,
                    pine: chart.pineOutput
                )
                .overlay(alignment: .trailing) {
                    PriceAxisRegion { preview.registerAxisRegion($0) }
                        .frame(width: ChartStyle.default.chartInsets.trailing)
                }
                if chart.showsPinePane {
                    PineScriptPaneView(pine: chart.pineOutput, candles: chart.visibleKlines, height: paneHeight)
                }
            }
        }
        .padding(6)
        .background(ZoomHitRegion { preview.registerZoomRegion($0) })
        .overlay { stateOverlay }
        .overlay(alignment: .top) { staleBanner }
        .overlay(alignment: .bottom) { errorBanner }
    }

    /// What replaces the chart when there is nothing to draw yet.
    @ViewBuilder private var stateOverlay: some View {
        if let reason = preview.unsupportedReason {
            message(symbol: "books.vertical", title: "Nothing to chart", detail: reason)
        } else if preview.market.source == .alpaca && !AlpacaCredentialsStore.isConfigured {
            message(
                symbol: "key", title: "Alpaca isn't set up",
                detail: "Stock candles come from Alpaca. Add your keys in Settings to test on stocks."
            ) {
                Button("Open Settings") {
                    UserDefaults.standard.set(SettingsTab.alpaca.rawValue, forKey: "settingsTab")
                    openSettings()
                }
            }
        } else if chart.klineData.isEmpty && chart.errorMessage == nil {
            VStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Loading \(chart.title)…").font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func message(symbol: String, title: String, detail: String) -> some View {
        message(symbol: symbol, title: title, detail: detail) { EmptyView() }
    }

    private func message<Action: View>(
        symbol: String, title: String, detail: String, @ViewBuilder action: () -> Action
    ) -> some View {
        VStack(spacing: 8) {
            Image(systemName: symbol).font(.title2).foregroundStyle(.secondary)
            Text(title).font(.subheadline.weight(.semibold))
            Text(detail)
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 260)
            action()
        }
        .padding()
    }

    /// The code in the editor doesn't compile; the chart still shows the last version that did.
    @ViewBuilder private var staleBanner: some View {
        if preview.isShowingStaleOutput {
            Button {
                withAnimation(.snappy) {
                    drawerTab = .problems
                    drawerOpen = true
                }
            } label: {
                Label(
                    "Fix the errors to update the chart — showing the last working version",
                    systemImage: "xmark.octagon.fill")
                    .font(.caption)
                    .lineLimit(1)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(.ultraThinMaterial, in: Capsule())
                    .overlay(Capsule().stroke(Color.red.opacity(0.5), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .padding(.top, 8)
            .help("Show problems")
            .transition(.move(edge: .top).combined(with: .opacity))
        }
    }

    @ViewBuilder private var errorBanner: some View {
        if let error = chart.errorMessage {
            HStack(spacing: 6) {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                Text(error).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                Button("Retry") { preview.retry() }
                    .font(.caption2)
                    .buttonStyle(.plain)
                    .foregroundStyle(.blue)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 4))
            .padding(.bottom, 8)
        }
    }
}
