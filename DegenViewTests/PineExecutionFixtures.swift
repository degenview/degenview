import Foundation

@testable import DegenView

/// Shared builders for the execution-model tests.
enum PineExecutionFixtures {
    static let barSeconds: TimeInterval = 60
    static let dataset = PineDatasetKey(symbolKey: "binance:BTC", timeframe: "1m")

    static func time(_ index: Int) -> Date { Date(timeIntervalSince1970: Double(index) * barSeconds) }

    /// A bar that opened at `open`, currently at `close`.
    static func bar(_ index: Int, open: Double, close: Double, closed: Bool = false) -> KlineData {
        .init(
            openTime: time(index), openPrice: open, highPrice: max(open, close) + 1,
            lowPrice: min(open, close) - 1, closePrice: close, volume: Double(index + 1), isClosed: closed)
    }

    /// Closed bars whose open equals their close, one per value.
    static func history(_ closes: [Double]) -> [KlineData] {
        closes.enumerated().map { bar($0.offset, open: $0.element, close: $0.element) }
    }

    static func program(_ body: String, header: String = "indicator(\"T\")") -> PineCompiledProgram {
        let program = PineCompiler.compile(source: "//@version=6\n\(header)\n\(body)")
        precondition(program.isValid, "\(program.diagnostics)")
        return program
    }

    static func controller(_ body: String, header: String = "indicator(\"T\")") -> PineExecutionController {
        PineExecutionController(program: program(body, header: header), dataset: dataset)
    }

    static func stream(_ bar: KlineData) -> PineMarketUpdate { .init(bar: bar, origin: .stream) }

    /// The realtime "now" for a bar: shortly after it opened, so it counts as still forming.
    static func now(during index: Int) -> Date { time(index).addingTimeInterval(10) }

    static func update(_ outcome: PineExecutionOutcome) -> PineExecutionUpdate? {
        if case .updated(let update) = outcome { return update }
        return nil
    }

    static func plot(_ output: PineVisualOutput, _ index: Int = 0) -> [Double?] {
        output.plots.sorted { $0.id < $1.id }[index].values
    }

    static func last(_ output: PineVisualOutput, _ index: Int = 0) -> Double? {
        plot(output, index).last ?? nil
    }
}
