import Foundation

/// Owns one `PineExecutionController` behind an actor, so a chart can feed it events from the main
/// actor while the script runs off it. Calls are serialized in arrival order.
actor PineExecutionHost {
    let dataset: PineDatasetKey
    private let controller: PineExecutionController

    init(
        program: PineCompiledProgram, dataset: PineDatasetKey, inputs: [String: PineInputValue],
        theme: PineChartTheme, symbol: PineSymbolInfo,
        securityData: PineSecurityDataProvider? = nil
    ) {
        self.dataset = dataset
        controller = PineExecutionController(
            program: program, dataset: dataset, inputs: inputs, theme: theme, symbol: symbol,
            securityData: securityData)
    }

    func rebuild(bars: [KlineData], live: Bool) -> PineExecutionOutcome {
        controller.rebuild(bars: bars, live: live)
    }

    func ingest(_ update: PineMarketUpdate) -> PineExecutionOutcome {
        controller.ingest(update)
    }

    func sync(snapshot: [KlineData]) -> PineExecutionOutcome {
        controller.sync(snapshot: snapshot)
    }
}
