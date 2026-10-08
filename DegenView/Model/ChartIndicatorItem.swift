import Foundation

/// A built-in indicator a chart can show. Each one is a single persisted flag on `ChartViewModel`
/// (there is no "hidden but applied" state), so adding turns the flag on and removing turns it off.
enum BuiltInIndicator: String, CaseIterable, Identifiable {
    case volume, rsi, ema, bollinger, trendFlips

    var id: String { rawValue }

    /// Name used in menus.
    var menuTitle: String {
        switch self {
        case .volume: "Volume"
        case .rsi: "RSI"
        case .ema: "EMA"
        case .bollinger: "Bollinger Bands"
        case .trendFlips: "Trend Flips"
        }
    }

    var icon: String {
        switch self {
        case .volume: "chart.bar.fill"
        case .rsi: "waveform.path.ecg"
        case .ema: "chart.line.uptrend.xyaxis"
        case .bollinger: "lines.measurement.horizontal"
        case .trendFlips: "arrow.triangle.2.circlepath"
        }
    }

    /// Whether the indicator has anything to configure.
    var hasSettings: Bool { self == .ema }

    enum Availability: Equatable {
        case available
        /// Does not apply to this chart at all, so it is not offered.
        case hidden
        /// Applies but cannot be turned on; `reason` says why.
        case unavailable(reason: String)
    }
}

@MainActor
extension BuiltInIndicator {
    /// The chip label, which carries the parameter that matters ("EMA 20").
    func chipTitle(in viewModel: ChartViewModel) -> String {
        switch self {
        case .volume: "Vol"
        case .rsi: "RSI \(RSI.period)"
        case .ema: "EMA \(viewModel.emaPeriod)"
        case .bollinger: "BB"
        case .trendFlips: "Trend Flips"
        }
    }

    func isOn(in viewModel: ChartViewModel) -> Bool {
        switch self {
        case .volume: viewModel.showVolume
        case .rsi: viewModel.showRSI
        case .ema: viewModel.showEMA
        case .bollinger: viewModel.showBollinger
        case .trendFlips: viewModel.showTrendFlips
        }
    }

    func isHidden(in viewModel: ChartViewModel) -> Bool { viewModel.hiddenBuiltIns.contains(self) }

    func setHidden(_ hidden: Bool, in viewModel: ChartViewModel) {
        if hidden { viewModel.hiddenBuiltIns.insert(self) } else { viewModel.hiddenBuiltIns.remove(self) }
    }

    func setOn(_ on: Bool, in viewModel: ChartViewModel) {
        switch self {
        case .volume: viewModel.showVolume = on
        case .rsi: viewModel.showRSI = on
        case .ema: viewModel.showEMA = on
        case .bollinger: viewModel.showBollinger = on
        case .trendFlips: viewModel.showTrendFlips = on
        }
    }

    /// Same rules as the settings sheet's Indicators tab.
    func availability(in viewModel: ChartViewModel) -> Availability {
        guard self == .volume else { return .available }
        if viewModel.usesLineChart { return .hidden }
        if !viewModel.source.providesVolume {
            return .unavailable(reason: "\(viewModel.source.displayName) doesn't report per-candle volume.")
        }
        return .available
    }
}

/// One entry of a chart's indicator strip: a built-in or an applied Pine script.
struct ChartIndicatorItem: Identifiable, Equatable {
    enum Kind: Equatable {
        case builtIn(BuiltInIndicator)
        case script(UUID)  // ChartScriptInstance.id
    }

    let kind: Kind
    let title: String
    let isVisible: Bool
    let hasError: Bool

    var id: String {
        switch kind {
        case .builtIn(let indicator): "builtin.\(indicator.rawValue)"
        case .script(let instanceID): "script.\(instanceID.uuidString)"
        }
    }

    var hasSettings: Bool {
        switch kind {
        case .builtIn(let indicator): indicator.hasSettings
        case .script: true
        }
    }
}

extension ChartIndicatorItem {
    /// The strip shows this many chips before folding the rest into a "+N" pill.
    static let visibleChipLimit = 3

    /// Splits `items` into the chips drawn inline and those folded into the overflow list.
    static func split(_ items: [ChartIndicatorItem]) -> (inline: [ChartIndicatorItem], overflow: [ChartIndicatorItem]) {
        (Array(items.prefix(visibleChipLimit)), Array(items.dropFirst(visibleChipLimit)))
    }
}

@MainActor
extension ChartViewModel {
    /// Everything applied to this chart, derived fresh — built-ins in a fixed order, then scripts in
    /// applied order. Never stored, so it cannot drift from the flags and instances it reads.
    var indicatorItems: [ChartIndicatorItem] {
        let builtIns = BuiltInIndicator.allCases.compactMap { indicator -> ChartIndicatorItem? in
            guard indicator.availability(in: self) != .hidden, indicator.isOn(in: self) else { return nil }
            return ChartIndicatorItem(
                kind: .builtIn(indicator), title: indicator.chipTitle(in: self),
                isVisible: !indicator.isHidden(in: self), hasError: false)
        }
        let scripts = pineLegendRows.map {
            ChartIndicatorItem(kind: .script($0.id), title: $0.title, isVisible: $0.isVisible, hasError: $0.hasError)
        }
        return builtIns + scripts
    }

    /// Whether `indicator` is applied and not switched off — what the chart actually draws.
    func isDrawn(_ indicator: BuiltInIndicator) -> Bool {
        indicator.isOn(in: self) && !indicator.isHidden(in: self)
    }

    /// Built-ins that could be added right now, with why a disabled one cannot be.
    var addableBuiltIns: [(indicator: BuiltInIndicator, availability: BuiltInIndicator.Availability)] {
        BuiltInIndicator.allCases.compactMap { indicator in
            let availability = indicator.availability(in: self)
            guard availability != .hidden, !indicator.isOn(in: self) else { return nil }
            return (indicator, availability)
        }
    }
}
