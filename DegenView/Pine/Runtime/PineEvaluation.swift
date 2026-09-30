import Foundation

/// The outcome of compiling a script and running it over a set of bars.
enum PineEvaluation: Sendable {
    case compileFailed(PineCompiledProgram)
    case runtimeFailed(PineCompiledProgram, PineDiagnostic)
    case applied(PineCompiledProgram, PineRuntimeResult)

    /// Compiles `source` (unless an already compiled program is supplied) and evaluates it.
    /// Blocking: call it from a background task.
    static func run(
        source: String, compiled supplied: PineCompiledProgram?, inputs: [String: PineInputValue],
        theme: PineChartTheme, symbol: PineSymbolInfo, bars: [KlineData]
    ) -> PineEvaluation {
        let compiled = supplied ?? PineCompiler.compile(source: source)
        guard compiled.isValid else { return .compileFailed(compiled) }
        do {
            let session = PineRuntimeSession(
                program: compiled, inputs: inputs, theme: theme, symbol: symbol)
            return .applied(compiled, try session.evaluate(bars: bars))
        } catch {
            let diagnostic =
                error as? PineDiagnostic
                ?? .error("PINE4999", .runtime, error.localizedDescription, .zero)
            return .runtimeFailed(compiled, diagnostic)
        }
    }
}
