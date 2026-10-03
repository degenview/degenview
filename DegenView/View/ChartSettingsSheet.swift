import AppKit
import SwiftUI

// MARK: - ChartSettingsSheet

/// Look and indicators for one chart. Every control writes through to the chart as it changes, so
/// there is nothing to save; a chart's market can't be changed here — remove it and add another.
struct ChartSettingsSheet: View {
    @ObservedObject var viewModel: ChartViewModel
    let onRemove: () -> Void
    let onStyleChanged: () -> Void

    @State private var selectedTab: Tab
    @State private var showRemoveConfirmation = false

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

    @Environment(\.dismiss) private var dismiss

    enum Tab: String, CaseIterable {
        case indicators = "Indicators"
        case scripts = "Scripts"
        case appearance = "Appearance"

        var systemImage: String {
            switch self {
            case .appearance: "paintpalette"
            case .indicators: "waveform.path.ecg"
            case .scripts: "curlybraces"
            }
        }
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

    init(
        viewModel: ChartViewModel,
        initialTab: Tab = .indicators,
        onRemove: @escaping () -> Void,
        onStyleChanged: @escaping () -> Void
    ) {
        self.viewModel = viewModel
        _selectedTab = State(initialValue: initialTab)
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
    }

    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(
                title: viewModel.title,
                subtitle: "\(viewModel.source.displayName) · Changes apply as you make them"
            ) {
                ChartIconView(viewModel: viewModel, size: 44)
                    .frame(width: 46, height: 46)
            }
            .padding(.horizontal, 24)
            .padding(.top, 24)
            .padding(.bottom, 16)

            IconTabBar(
                items: Tab.allCases.map { .init(value: $0, title: $0.rawValue, systemImage: $0.systemImage) },
                selection: $selectedTab
            )
            .padding(.bottom, 16)

            Divider()

            Group {
                switch selectedTab {
                case .appearance: appearanceTab
                case .indicators: indicatorsTab
                case .scripts: scriptsTab
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)

            Divider()
            footer
        }
        .frame(
            minWidth: UI.chartSettingsSheetMinWidth,
            idealWidth: UI.chartSettingsSheetWidth,
            minHeight: UI.chartSettingsSheetMinHeight,
            idealHeight: UI.chartSettingsSheetHeight
        )
        .background {
            WindowAccessor { window in
                window.styleMask.insert(.resizable)
                window.minSize = NSSize(
                    width: UI.chartSettingsSheetMinWidth,
                    height: UI.chartSettingsSheetMinHeight
                )
            }
        }
        .confirmationDialog(
            "Remove \(viewModel.title)?", isPresented: $showRemoveConfirmation, titleVisibility: .visible
        ) {
            Button("Remove Chart", role: .destructive) {
                dismiss()
                onRemove()
            }
        } message: {
            Text(
                "The chart leaves this tab; its drawings are kept. "
                    + "To change a chart's market, remove it and add a new one."
            )
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
    }

    // MARK: - Footer

    private var footer: some View {
        HStack {
            Button(role: .destructive) {
                showRemoveConfirmation = true
            } label: {
                Label("Remove Chart", systemImage: "trash")
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

    /// A settings page: titled sections of cards, scrolling when the sheet is short.
    private func page<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) { content() }
                .padding(24)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func section<Content: View>(
        _ title: String, subtitle: String? = nil, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.title3.weight(.semibold))
                if let subtitle {
                    Text(subtitle).font(.subheadline).foregroundStyle(.secondary)
                }
            }
            content()
        }
    }

    // MARK: - Appearance Tab

    private var appearanceTab: some View {
        page {
            section("Colors", subtitle: "Used for rising and falling prices.") {
                SettingsCardRow(
                    title: viewModel.usesLineChart ? "Line up" : "Bullish candle", icon: "arrow.up.right",
                    hint: viewModel.usesLineChart
                        ? "Stretches where the price rises." : "Price closed higher than it opened."
                ) {
                    ColorPicker("", selection: $bullishColor).labelsHidden()
                }
                SettingsCardRow(
                    title: viewModel.usesLineChart ? "Line down" : "Bearish candle", icon: "arrow.down.right",
                    hint: viewModel.usesLineChart
                        ? "Stretches where the price falls." : "Price closed lower than it opened."
                ) {
                    ColorPicker("", selection: $bearishColor).labelsHidden()
                }
                HStack(spacing: 16) {
                    CandleColorPreview(bullish: bullishColor, bearish: bearishColor, isLine: viewModel.usesLineChart)
                        .padding(10)
                        .background(
                            .background.opacity(0.6), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Preview").font(.subheadline.weight(.semibold))
                        Text("The chart behind this window updates as you pick.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 16)
                    Button("Reset Colors") {
                        bullishColor = .green
                        bearishColor = .red
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 12))
                .overlay { RoundedRectangle(cornerRadius: 12).stroke(.separator.opacity(0.45), lineWidth: 1) }
            }

            section("Price axis") {
                SettingsCardRow(title: "Decimal places", icon: "number", hint: decimalHint) {
                    Picker("Decimals", selection: $decimalPlacesMode) {
                        ForEach(DecimalMode.allCases) { mode in
                            Text(mode.rawValue).tag(mode)
                        }
                    }
                    .pickerStyle(.menu)
                    .labelsHidden()
                    .frame(width: 90)
                }
            }
        }
    }

    // MARK: - Indicators Tab

    private var indicatorsTab: some View {
        page {
            section("Indicators", subtitle: "Add context to the chart without changing its underlying data.") {
                // Line sources report one price per timestamp and no turnover at all.
                if !viewModel.usesLineChart {
                    SettingsCardRow(
                        title: "Volume Bars",
                        icon: "chart.bar.fill",
                        hint: volumeHint
                    ) {
                        Toggle("Volume Bars", isOn: $showVolume)
                            .labelsHidden()
                            .toggleStyle(.switch)
                            // Left visible rather than hidden: a greyed-out switch with a
                            // reason reads better than a setting that silently does nothing.
                            .disabled(!viewModel.source.providesVolume)
                    }
                }

                // Every source shares KlineData. Line sources carry flat OHLC points,
                // whose point-to-point gaps still provide Supertrend's true range.
                SettingsCardRow(title: "RSI (\(RSI.period))", icon: "waveform.path.ecg", hint: rsiHint) {
                    Toggle("RSI (\(RSI.period))", isOn: $showRSI)
                        .labelsHidden()
                        .toggleStyle(.switch)
                }

                SettingsCardRow(title: "EMA", icon: "chart.line.uptrend.xyaxis", hint: emaHint) {
                    HStack(spacing: 10) {
                        Picker("Period", selection: $emaPeriod) {
                            ForEach(Indicator.emaPeriods, id: \.self) { period in
                                Text("\(period)").tag(period)
                            }
                        }
                        .pickerStyle(.menu)
                        .labelsHidden()
                        .frame(width: 72)
                        .disabled(!showEMA)

                        Toggle("EMA", isOn: $showEMA)
                            .labelsHidden()
                            .toggleStyle(.switch)
                    }
                }

                SettingsCardRow(title: "Bollinger Bands", icon: "lines.measurement.horizontal", hint: bollingerHint) {
                    Toggle("Bollinger Bands", isOn: $showBollinger)
                        .labelsHidden()
                        .toggleStyle(.switch)
                }

                SettingsCardRow(
                    title: "Trend Flips",
                    icon: "arrow.triangle.2.circlepath",
                    hint: trendFlipsHint
                ) {
                    Toggle("Trend Flips (Supertrend)", isOn: $showTrendFlips)
                        .labelsHidden()
                        .toggleStyle(.switch)
                }
            }
        }
    }

    // MARK: - Scripts Tab

    private var scriptsTab: some View {
        ChartScriptsTab(viewModel: viewModel, onStyleChanged: onStyleChanged)
    }

    private var volumeHint: String {
        guard viewModel.source.providesVolume else {
            return
                "\(viewModel.source.displayName) doesn't report per-candle volume, so bars aren't available for this chart."
        }
        return "Turnover per candle, drawn along the bottom of the chart."
    }

    /// Flags the warm-up explicitly: RSI has no value for its first `period` candles,
    /// so a short range draws a stub of a line or none at all.
    private var rsiHint: String {
        let loaded = viewModel.klineData.count
        if loaded > 0 && loaded <= RSI.period {
            return
                "Only \(loaded) candles loaded — RSI needs more than \(RSI.period). Zoom out or pick a longer range."
        }
        return
            "Momentum from 0–100 across the bottom, with guides at \(Int(RSI.oversold)) and \(Int(RSI.overbought))."
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
        onRemove: {},
        onStyleChanged: {}
    )
}
