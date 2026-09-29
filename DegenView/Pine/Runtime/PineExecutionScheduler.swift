import Foundation

/// Decides WHEN a script runs; `PineRuntimeSession` decides HOW.
enum PineExecutionScheduler {
    /// Whether `declaration`'s script executes for a bar event in `phase`.
    ///
    /// Indicators and libraries run on every event. A strategy runs on history and on the closing
    /// update of each realtime bar, plus on every realtime tick when `calc_on_every_tick` is set.
    static func shouldExecute(_ declaration: PineDeclarationMetadata, phase: PineBarPhase) -> Bool {
        guard declaration.type == .strategy else { return true }
        if !phase.isRealtime || phase.isConfirmed { return true }
        return declaration.strategy?.calcOnEveryTick == true
    }
}
