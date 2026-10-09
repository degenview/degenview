import Foundation

/// Gets a script the series its `request.security` calls read: another symbol, or the chart's own on a longer
/// timeframe with history from before the chart's first bar.
///
/// The engine only reads what it is handed, and what a script asks for can depend on its inputs, so the script
/// is first run once against a recorder that answers "no candles" to learn the series, then those are fetched.
/// A series that cannot be fetched is left out, and the session then reports it (another symbol) or builds the
/// series from the chart's bars (the chart's own symbol).
///
/// The `request.security` result is a snapshot taken when the script is built: it does not tick with the
/// market. A `request.security_lower_tf` series is a `PineIntrabarSeries`, which the chart tops up on each refresh.
enum PineSecurityFeed {
    /// Candles asked for beyond what the chart's span needs, so an expression's own lookback is warm.
    static let warmupCandles = 300
    static let maximumCandles = 1_000
    /// A slow source delays the script, not the chart; after this the series that are in are used.
    static let timeout: TimeInterval = 8

    static func prepare(
        program: PineCompiledProgram, inputs: [String: PineInputValue], theme: PineChartTheme,
        symbol: PineSymbolInfo, chart: PineSecurityTarget.Chart, bars: [KlineData]
    ) async -> PineSecurityDataProvider? {
        guard usesSecurity(program), !bars.isEmpty else { return nil }
        let recorder = PineSecurityRecorder()
        // Errors do not matter here: the series asked for before the error are the ones recorded.
        _ = try? PineRuntimeSession(
            program: program, inputs: inputs, theme: theme, symbol: symbol, securityData: recorder
        ).evaluate(bars: bars)
        let spacing = bars.count > 1 ? bars[1].openTime.timeIntervalSince(bars[0].openTime) : 0
        let span = Double(bars.count) * spacing
        var wanted: [(PineSecurityKey, PineSecurityTarget)] = []
        var intrabars: [PineSecurityKey] = []
        for key in recorder.keys {
            // The chart's own symbol on a bar length shorter than the chart's is `request.security_lower_tf`.
            if spacing > 0, key.interval < spacing, key.symbol == chart.tickerID {
                intrabars.append(key)
                continue
            }
            guard let target = PineSecurityTarget.resolve(key, chart: chart) else { continue }
            wanted.append((key, target))
        }
        guard !wanted.isEmpty || !intrabars.isEmpty else { return nil }
        let fetched = wanted.isEmpty ? PineFetchedSecurityData() : await fetch(wanted, chartSpan: span)
        guard !intrabars.isEmpty else { return fetched.series.isEmpty ? nil : fetched }
        let series = PineIntrabarSeries(security: fetched, source: chart.source, apiSymbol: chart.apiSymbol)
        await series.load(intrabars, bars: bars, spacing: spacing)
        return series.isEmpty ? nil : series
    }

    /// Whether the script, or a library it imports, calls `request.security`.
    static func usesSecurity(_ program: PineCompiledProgram) -> Bool {
        program.source.contains("request.security")
            || program.imports.contains { $0.library.map(usesSecurity) ?? false }
    }

    private static func fetch(
        _ wanted: [(PineSecurityKey, PineSecurityTarget)], chartSpan: TimeInterval
    ) async -> PineFetchedSecurityData {
        await withTaskGroup(of: Fetched.self) { group in
            for (key, target) in wanted {
                group.addTask {
                    let covering = Int((chartSpan / target.range.binanceIntervalSeconds).rounded(.up))
                    let limit = min(maximumCandles, covering + warmupCandles)
                    let service = DataSourceFactory.shared.service(for: target.source)
                    guard
                        let candles = try? await service.fetchKlines(
                            symbol: target.symbol, interval: target.range.binanceInterval, limit: limit),
                        !candles.isEmpty
                    else { return .missing }
                    return .series(key, candles.sorted { $0.openTime < $1.openTime })
                }
            }
            group.addTask {
                try? await Task.sleep(for: .seconds(timeout))
                return .timedOut
            }
            var data = PineFetchedSecurityData()
            var finished = 0
            collecting: for await result in group {
                switch result {
                case .series(let key, let candles):
                    data.series[key] = candles
                    finished += 1
                case .missing: finished += 1
                case .timedOut: break collecting
                }
                if finished >= wanted.count { break }
            }
            group.cancelAll()
            return data
        }
    }

    private enum Fetched {
        case series(PineSecurityKey, [KlineData])
        case missing
        case timedOut
    }
}
